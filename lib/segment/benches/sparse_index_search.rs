#[cfg(not(target_os = "windows"))]
mod prof;

use std::sync::atomic::AtomicBool;

use common::types::PointOffsetType;
use criterion::{criterion_group, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::SeedableRng;
use segment::fixtures::sparse_fixtures::fixture_sparse_index_ram;
use segment::index::{PayloadIndex, VectorIndex};
use segment::types::PayloadSchemaType::Keyword;
use segment::types::{Condition, FieldCondition, Filter, Payload};
use serde_json::json;
use sparse::common::sparse_vector_fixture::random_sparse_vector;
use tempfile::Builder;

const NUM_VECTORS: usize = 50_000;
const MAX_SPARSE_DIM: usize = 30_000;
const TOP: usize = 10;
const FULL_SCAN_THRESHOLD: usize = 1; // low value to trigger index usage by default

fn sparse_vector_index_search_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparse-vector-search-group");

    let stopped = AtomicBool::new(false);
    let mut rnd = StdRng::seed_from_u64(42);

    let data_dir = Builder::new().prefix("data_dir").tempdir().unwrap();
    let sparse_vector_index = fixture_sparse_index_ram(
        &mut rnd,
        NUM_VECTORS,
        MAX_SPARSE_DIM,
        FULL_SCAN_THRESHOLD,
        data_dir.path(),
        &stopped,
    );

    // adding payload on field
    let field_name = "field";
    let field_value = "important value";
    let payload: Payload = json!({
        field_name: field_value,
    })
    .into();

    // all points have the same payload
    let mut payload_index = sparse_vector_index.payload_index.borrow_mut();
    for idx in 0..NUM_VECTORS {
        payload_index
            .assign(idx as PointOffsetType, &payload)
            .unwrap();
    }
    drop(payload_index);

    // shared query vector
    let sparse_vector = random_sparse_vector(&mut rnd, MAX_SPARSE_DIM);
    eprintln!("sparse_vector size = {:#?}", sparse_vector.values.len());
    let query_vector = sparse_vector.into();

    // intent: bench `search` without filter
    group.bench_function("inverted-index", |b| {
        b.iter(|| {
            let results = sparse_vector_index
                .search(&[&query_vector], None, TOP, None, &stopped)
                .unwrap();

            assert_eq!(results[0].len(), TOP);
        })
    });

    // filter by field
    let filter = Filter::new_must(Condition::Field(FieldCondition::new_match(
        field_name,
        field_value.to_owned().into(),
    )));

    // intent: bench `search` when the filtered payload key is not indexed
    group.bench_function("inverted-index-filtered-plain", |b| {
        b.iter(|| {
            let results = sparse_vector_index
                .search(&[&query_vector], Some(&filter), TOP, None, &stopped)
                .unwrap();

            assert_eq!(results[0].len(), TOP);
        })
    });

    // intent: bench `search_plain` when the filtered payload key is not indexed
    group.bench_function("plain-storage", |b| {
        b.iter(|| {
            let results = sparse_vector_index
                .search_plain(&[&query_vector], &filter, TOP, &stopped)
                .unwrap();

            assert_eq!(results[0].len(), TOP);
        })
    });

    let mut payload_index = sparse_vector_index.payload_index.borrow_mut();

    // create payload field index
    payload_index
        .set_indexed(field_name, Keyword.into())
        .unwrap();

    drop(payload_index);

    // intent: bench `search` when the filterer payload key is indexed
    group.bench_function("inverted-index-filtered-payload-index", |b| {
        b.iter(|| {
            let results = sparse_vector_index
                .search(&[&query_vector], Some(&filter), TOP, None, &stopped)
                .unwrap();

            assert_eq!(results[0].len(), TOP);
        })
    });

    // intent: bench `search_plain` when the filterer payload key is indexed
    group.bench_function("payload-index", |b| {
        b.iter(|| {
            let results = sparse_vector_index
                .search_plain(&[&query_vector], &filter, TOP, &stopped)
                .unwrap();

            assert_eq!(results[0].len(), TOP);
        })
    });

    group.finish();
}

#[cfg(not(target_os = "windows"))]
criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(prof::FlamegraphProfiler::new(100));
    targets = sparse_vector_index_search_benchmark
}

#[cfg(target_os = "windows")]
criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = sparse_vector_index_search_benchmark,
}

criterion_main!(benches);
