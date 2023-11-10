use std::collections::HashSet;
use std::fs::create_dir_all;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use atomic_refcell::AtomicRefCell;
use common::types::{PointOffsetType, ScoredPointOffset};
use sparse::common::sparse_vector::SparseVector;
use sparse::index::inverted_index::inverted_index_ram::InvertedIndexRam;
use sparse::index::inverted_index::InvertedIndex;
use sparse::index::search_context::SearchContext;

use crate::common::operation_error::{check_process_stopped, OperationError, OperationResult};
use crate::common::operation_time_statistics::ScopeDurationMeasurer;
use crate::data_types::vectors::QueryVector;
use crate::id_tracker::IdTrackerSS;
use crate::index::query_estimator::adjust_to_available_vectors;
use crate::index::sparse_index::sparse_index_config::SparseIndexConfig;
use crate::index::sparse_index::sparse_search_telemetry::SparseSearchesTelemetry;
use crate::index::struct_payload_index::StructPayloadIndex;
use crate::index::{PayloadIndex, VectorIndex};
use crate::telemetry::VectorIndexSearchesTelemetry;
use crate::types::{Filter, SearchParams};
use crate::vector_storage::sparse_raw_scorer::sparse_check_vector;
use crate::vector_storage::{new_stoppable_raw_scorer, VectorStorage, VectorStorageEnum};

pub struct SparseVectorIndex<TInvertedIndex: InvertedIndex> {
    pub config: SparseIndexConfig,
    pub id_tracker: Arc<AtomicRefCell<IdTrackerSS>>,
    pub vector_storage: Arc<AtomicRefCell<VectorStorageEnum>>,
    pub payload_index: Arc<AtomicRefCell<StructPayloadIndex>>,
    path: PathBuf,
    pub inverted_index: TInvertedIndex,
    searches_telemetry: SparseSearchesTelemetry,
    max_point_id: PointOffsetType, // used to compute the number of indexed vectors
}

impl<TInvertedIndex: InvertedIndex> SparseVectorIndex<TInvertedIndex> {
    /// Open a sparse vector index at a given path
    pub fn open(
        config: SparseIndexConfig,
        id_tracker: Arc<AtomicRefCell<IdTrackerSS>>,
        vector_storage: Arc<AtomicRefCell<VectorStorageEnum>>,
        payload_index: Arc<AtomicRefCell<StructPayloadIndex>>,
        path: &Path,
    ) -> OperationResult<Self> {
        // create directory if it does not exist
        create_dir_all(path)?;

        // load config
        let config_path = SparseIndexConfig::get_config_path(path);
        let config = if config_path.exists() {
            SparseIndexConfig::load(path)?
        } else {
            // use provided config if no config file exists
            config
        };

        let searches_telemetry = SparseSearchesTelemetry::new();
        let max_point_id = 0;
        let inverted_index = TInvertedIndex::open(path)?;
        let path = path.to_path_buf();
        let index = Self {
            config,
            id_tracker,
            vector_storage,
            payload_index,
            path,
            inverted_index,
            searches_telemetry,
            max_point_id,
        };
        Ok(index)
    }

    fn save_config(&self) -> OperationResult<()> {
        let config_path = SparseIndexConfig::get_config_path(&self.path);
        self.config.save(&config_path)
    }

    /// Search index using sparse vector query
    pub fn search_sparse(
        &self,
        vectors: &[&QueryVector],
        top: usize,
        is_stopped: &AtomicBool,
        with_filter: bool,
        condition: impl Fn(PointOffsetType) -> bool,
    ) -> OperationResult<Vec<Vec<ScoredPointOffset>>> {
        let mut result = Vec::with_capacity(vectors.len());

        for vector in vectors {
            check_process_stopped(is_stopped)?;
            // measure time according to filter
            let _timer = if with_filter {
                ScopeDurationMeasurer::new(&self.searches_telemetry.filtered_sparse)
            } else {
                ScopeDurationMeasurer::new(&self.searches_telemetry.unfiltered_sparse)
            };
            let vector = match vector {
                QueryVector::Nearest(vector) => vector,
                QueryVector::Recommend(_) => {
                    return Err(OperationError::ValidationError {
                        description: "Cannot recommend sparse vectors".to_string(),
                    })
                }
                QueryVector::Discovery(_) => {
                    return Err(OperationError::ValidationError {
                        description: "Cannot discovery sparse vectors".to_string(),
                    })
                }
                QueryVector::Context(_) => {
                    return Err(OperationError::ValidationError {
                        description: "Cannot context query sparse vectors".to_string(),
                    })
                }
            };
            let sparse_vector: SparseVector = vector.clone().try_into()?;
            let mut search_context =
                SearchContext::new(sparse_vector, top, &self.inverted_index, is_stopped);
            let points = search_context.search(&condition);
            result.push(points);
        }

        Ok(result)
    }

    /// Returns the maximum number of results that can be returned by the index for a given sparse vector
    /// Warning: the cost of this function grows with the number of dimensions in the query vector
    pub fn max_result_count(&self, query_vector: &SparseVector) -> usize {
        let mut unique_record_ids = HashSet::new();
        for dim_id in query_vector.indices.iter() {
            if let Some(posting_list) = self.inverted_index.get(dim_id) {
                for element in posting_list.elements.iter() {
                    unique_record_ids.insert(element.record_id);
                }
            }
        }
        unique_record_ids.len()
    }

    /// Plain search not using the inverted index.
    pub fn search_plain(
        &self,
        vectors: &[&QueryVector],
        filter: &Filter,
        top: usize,
        is_stopped: &AtomicBool,
    ) -> OperationResult<Vec<Vec<ScoredPointOffset>>> {
        let mut results = Vec::with_capacity(vectors.len());
        let id_tracker = self.id_tracker.borrow();
        let payload_index = self.payload_index.borrow();
        let vector_storage = &self.vector_storage.borrow();
        // run filter once for all vectors
        let filtered_points = payload_index.query_points(filter);
        for &vector in vectors {
            check_process_stopped(is_stopped)?;
            let _timer = ScopeDurationMeasurer::new(&self.searches_telemetry.unfiltered_sparse);
            let raw_scorer = new_stoppable_raw_scorer(
                vector.clone(),
                vector_storage,
                id_tracker.deleted_point_bitslice(),
                is_stopped,
            )?;
            let search_results =
                raw_scorer.peek_top_iter(&mut filtered_points.iter().copied(), top);
            results.push(search_results);
        }
        Ok(results)
    }
}

impl<TInvertedIndex: InvertedIndex> VectorIndex for SparseVectorIndex<TInvertedIndex> {
    fn search(
        &self,
        vectors: &[&QueryVector],
        filter: Option<&Filter>,
        top: usize,
        _params: Option<&SearchParams>, // unused for sparse search
        is_stopped: &AtomicBool,
    ) -> OperationResult<Vec<Vec<ScoredPointOffset>>> {
        let id_tracker = self.id_tracker.borrow();
        let vector_storage = self.vector_storage.borrow();
        let deleted_point_bitslice = id_tracker.deleted_point_bitslice();
        let deleted_vectors = vector_storage.deleted_vector_bitslice();
        // filter for deleted points
        let not_deleted_condition = |idx: PointOffsetType| -> bool {
            sparse_check_vector(idx, deleted_point_bitslice, deleted_vectors)
        };
        match filter {
            Some(filter) => {
                let payload_index = self.payload_index.borrow();
                let id_tracker = self.id_tracker.borrow();
                let available_vector_count = vector_storage.available_vector_count();
                let query_point_cardinality = payload_index.estimate_cardinality(filter);
                let query_cardinality = adjust_to_available_vectors(
                    query_point_cardinality,
                    available_vector_count,
                    id_tracker.available_point_count(),
                );

                // if cardinality is small - use plain search
                if query_cardinality.max < self.config.full_scan_threshold {
                    let _timer =
                        ScopeDurationMeasurer::new(&self.searches_telemetry.small_cardinality);
                    return self.search_plain(vectors, filter, top, is_stopped);
                }

                let filter_context = payload_index.filter_context(filter);
                let matches_filter_condition =
                    |idx: PointOffsetType| -> bool { filter_context.check(idx) };
                self.search_sparse(vectors, top, is_stopped, true, |idx| {
                    not_deleted_condition(idx) && matches_filter_condition(idx)
                })
            }
            None => {
                // query sparse index directly
                self.search_sparse(vectors, top, is_stopped, false, not_deleted_condition)
            }
        }
    }

    fn build_index(&mut self, stopped: &AtomicBool) -> OperationResult<()> {
        let borrowed_vector_storage = self.vector_storage.borrow();
        let borrowed_id_tracker = self.id_tracker.borrow();
        let deleted_bitslice = borrowed_vector_storage.deleted_vector_bitslice();
        let mut ram_index = InvertedIndexRam::empty();
        let mut index_point_count: usize = 0;
        for id in borrowed_id_tracker.iter_ids_excluding(deleted_bitslice) {
            check_process_stopped(stopped)?;
            let vector: &SparseVector = borrowed_vector_storage.get_vector(id).try_into()?;
            ram_index.upsert(id, vector.to_owned());
            index_point_count += 1;
        }
        self.max_point_id = index_point_count.saturating_sub(1) as PointOffsetType;
        // TODO(sparse) this operation loads the entire index into memory which can cause OOM on large storage
        self.inverted_index = TInvertedIndex::from_ram_index(ram_index, &self.path)?;

        // save config to mark successful build
        self.save_config()?;
        Ok(())
    }

    fn get_telemetry_data(&self) -> VectorIndexSearchesTelemetry {
        let tm = &self.searches_telemetry;
        tm.into()
    }

    fn files(&self) -> Vec<PathBuf> {
        self.inverted_index.files()
    }

    fn indexed_vector_count(&self) -> usize {
        // internal ids start at 0
        self.max_point_id as usize + 1
    }

    fn update_vector(&mut self, id: PointOffsetType) -> OperationResult<()> {
        let vector_storage = self.vector_storage.borrow();
        let vector: &SparseVector = vector_storage.get_vector(id).try_into()?;
        // there are no holes in the internal ids, so we can just use the id as the count
        self.max_point_id = self.max_point_id.max(id);
        self.inverted_index.upsert(id, vector.clone());
        Ok(())
    }
}
