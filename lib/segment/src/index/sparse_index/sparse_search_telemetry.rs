use std::sync::Arc;

use parking_lot::Mutex;

use crate::common::operation_time_statistics::OperationDurationsAggregator;
use crate::telemetry::VectorIndexSearchesTelemetry;

pub struct SparseSearchesTelemetry {
    pub filtered_sparse: Arc<Mutex<OperationDurationsAggregator>>,
    pub unfiltered_sparse: Arc<Mutex<OperationDurationsAggregator>>,
    pub filtered_plain: Arc<Mutex<OperationDurationsAggregator>>,
    pub small_cardinality: Arc<Mutex<OperationDurationsAggregator>>,
}

impl SparseSearchesTelemetry {
    pub fn new() -> Self {
        SparseSearchesTelemetry {
            filtered_sparse: OperationDurationsAggregator::new(),
            unfiltered_sparse: OperationDurationsAggregator::new(),
            filtered_plain: OperationDurationsAggregator::new(),
            small_cardinality: OperationDurationsAggregator::new(),
        }
    }
}

impl Default for SparseSearchesTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&SparseSearchesTelemetry> for VectorIndexSearchesTelemetry {
    fn from(value: &SparseSearchesTelemetry) -> Self {
        VectorIndexSearchesTelemetry {
            index_name: None,
            unfiltered_plain: Default::default(),
            filtered_plain: value.filtered_plain.lock().get_statistics(),
            unfiltered_hnsw: Default::default(),
            filtered_small_cardinality: value.small_cardinality.lock().get_statistics(),
            filtered_large_cardinality: Default::default(),
            filtered_exact: Default::default(),
            filtered_sparse: value.filtered_sparse.lock().get_statistics(),
            unfiltered_sparse: value.unfiltered_sparse.lock().get_statistics(),
            unfiltered_exact: Default::default(),
        }
    }
}
