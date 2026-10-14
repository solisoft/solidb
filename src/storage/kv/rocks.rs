//! RocksDB under [`Kv`]: how SoliDB configures and opens it. The only place
//! besides `kv/mod.rs` that names `rust_rocksdb`, legacy-layout tests aside.

use std::path::Path;
use std::sync::OnceLock;

use rust_rocksdb::{BlockBasedOptions, Cache, ColumnFamilyDescriptor, DBCompressionType, Options};

use super::{Kv, KvError, KvMemory, RawRocks};
use crate::storage::engine::profile;

/// Shared block cache used by all column families (and all DB instances).
/// Without an explicit table factory, each CF gets its own private default
/// cache and no bloom filter — with thousands of CFs that wastes memory and
/// bypasses the cache/bloom tuning entirely.
fn shared_block_cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Cache::new_lru_cache(profile().block_cache_bytes))
}

/// Create optimized column family options
/// Used for ALL column families — including those created via `Database` —
/// to ensure consistent compression, caching, and performance settings
pub(super) fn tuned_cf_options() -> Options {
    let p = profile();
    let mut opts = Options::default();

    // Enable LZ4 compression for this column family
    opts.set_compression_type(DBCompressionType::Lz4);

    // Level compaction is the default and works well for most workloads
    // Optimize for SSD storage with fast sequential I/O
    opts.set_target_file_size_base(64 * 1024 * 1024); // 64MB base file size
    opts.set_target_file_size_multiplier(2);

    // Write buffer settings (per-CF memtable; profile-tuned)
    opts.set_write_buffer_size(p.write_buffer_size);
    opts.set_max_write_buffer_number(p.max_write_buffer_number);
    opts.set_min_write_buffer_number_to_merge(1);
    // One arena block is reserved per CF on its first write, so this is paid
    // by every collection that exists rather than by every byte stored.
    opts.set_arena_block_size(p.arena_block_size);

    // Optimize for SSD storage - parallel compactions
    opts.set_max_subcompactions(4);

    // Shared block cache + bloom filter for faster point lookups
    let mut block_opts = BlockBasedOptions::default();
    block_opts.set_block_cache(shared_block_cache());
    block_opts.set_bloom_filter(10.0, false);
    if p.cache_index_and_filter_blocks {
        // Bound index/filter RAM by storing those blocks in the shared cache
        // rather than pinning them per-CF (matters with thousands of CFs).
        block_opts.set_cache_index_and_filter_blocks(true);
        block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
    }
    opts.set_block_based_table_factory(&block_opts);

    opts
}

/// Open (or create) the main database at `path` with every existing column
/// family, creating those in `required` that are missing. Tuned by the
/// process-wide [`crate::storage::engine::EngineProfile`].
pub(crate) fn open(path: &Path, required: &[&str]) -> Result<Kv, KvError> {
    // Configure RocksDB options with performance optimizations
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    // Compression settings - use LZ4 for fast compression/decompression
    // Reduces storage size by ~40-60% and improves I/O performance
    opts.set_compression_type(DBCompressionType::Lz4);
    opts.set_compression_options(-14, -1, 0, 0);

    // Block cache - shared 512MB cache across all CFs and DB instances
    // Improves read performance by caching frequently accessed blocks
    let mut block_opts = BlockBasedOptions::default();
    block_opts.set_block_cache(shared_block_cache());
    // Enable bloom filter for faster point lookups
    block_opts.set_bloom_filter(10.0, false);
    opts.set_block_based_table_factory(&block_opts);

    let p = profile();

    // Write buffer settings - larger memtable reduces flush frequency
    // Better for write-heavy workloads (profile-tuned)
    opts.set_write_buffer_size(p.write_buffer_size);
    opts.set_max_write_buffer_number(p.max_write_buffer_number + 1);
    opts.set_min_write_buffer_number_to_merge(1);
    opts.set_arena_block_size(p.arena_block_size);

    // Global cap on total memtable memory across ALL column families.
    // The single most effective knob when CF count is large: without it,
    // memtable RAM scales with the number of CFs.
    if let Some(budget) = p.db_write_buffer_size {
        opts.set_db_write_buffer_size(budget);
    }

    // Bound the table cache (pinned index/filter blocks scale with open
    // SST files, which scale with CF count).
    opts.set_max_open_files(p.max_open_files);

    // Background jobs - more threads for compaction/flushing
    // Improves write throughput under heavy load (profile-tuned)
    opts.set_max_background_jobs(p.max_background_jobs);
    opts.set_max_subcompactions(4);

    // Target file size for better compaction behavior
    opts.set_target_file_size_base(64 * 1024 * 1024); // 64MB
    opts.set_target_file_size_multiplier(2);

    // Level compaction settings
    opts.set_max_bytes_for_level_base(512 * 1024 * 1024); // 512MB
    opts.set_max_bytes_for_level_multiplier(10.0);
    opts.set_num_levels(7);

    // Total WAL budget across all column families (profile-tuned).
    // This is a flush *trigger*, not just a disk cap — see the field's
    // documentation on `EngineProfile` for why a small value costs more
    // than it saves once the CF count is in the hundreds.
    opts.set_max_total_wal_size(p.max_total_wal_size as u64);

    // Keep fewer LOG files (RocksDB info logs, not WALs). The count is
    // bounded here and the size below: `keep_log_file_num` alone let
    // data/LOG reach 118MB on an instance flushing 700 CFs at a time.
    opts.set_keep_log_file_num(5);
    opts.set_max_log_file_size(64 * 1024 * 1024);

    // Recycle WAL files instead of deleting them
    opts.set_recycle_log_file_num(3);

    // Enable parallel memtable writes for better concurrency
    opts.set_enable_pipelined_write(true);

    // Existing column families, plus the ones this instance needs.
    let mut cf_names =
        RawRocks::list_cf(&opts, path).unwrap_or_else(|_| vec!["default".to_string()]);
    for name in required {
        if !cf_names.iter().any(|n| n == name) {
            cf_names.push((*name).to_string());
        }
    }

    // Every column family gets the same tuned options.
    let cf_descriptors: Vec<ColumnFamilyDescriptor> = cf_names
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(name, tuned_cf_options()))
        .collect();

    Ok(Kv::from(RawRocks::open_cf_descriptors(
        &opts,
        path,
        cf_descriptors,
    )?))
}

/// Open (or create) the replication log's database: default column family
/// only, sized for an append-mostly log.
pub(crate) fn open_sync_log(path: &Path) -> Result<Kv, KvError> {
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_max_write_buffer_number(4);
    opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
    opts.set_keep_log_file_num(5);
    opts.set_recycle_log_file_num(3);
    // Listing the default family is what gives `Kv::default_cf` a handle.
    let families = [rust_rocksdb::DEFAULT_COLUMN_FAMILY_NAME];
    Ok(Kv::from(RawRocks::open_cf(&opts, path, families)?))
}

/// Memory figures summed over `cfs`. The block cache is one shared LRU
/// (`shared_block_cache`) that every column family reports, so it is read
/// from one handle only.
pub(super) fn memory(db: &RawRocks, cfs: &[String]) -> KvMemory {
    let read = |cf: &std::sync::Arc<rust_rocksdb::BoundColumnFamily<'_>>, prop: &str| -> u64 {
        db.property_int_value_cf(cf, prop)
            .ok()
            .flatten()
            .unwrap_or(0)
    };
    let mut out = KvMemory::default();
    let mut first = None;
    for name in cfs {
        let Some(cf) = db.cf_handle(name) else {
            continue;
        };
        // Live memtables, and live + immutable ones still awaiting flush.
        // The gap between the two is flush backlog.
        out.memtable_bytes += read(&cf, "rocksdb.cur-size-all-mem-tables");
        out.memtable_total_bytes += read(&cf, "rocksdb.size-all-mem-tables");
        // Index and filter blocks of every open SST. With
        // `cache_index_and_filter_blocks` off and `max_open_files` at -1
        // (both the prod defaults) this sits *outside* the block cache and
        // is never evicted, so it is the counter that grows with the
        // dataset rather than with write traffic.
        out.table_readers_bytes += read(&cf, "rocksdb.estimate-table-readers-mem");
        for level in 0..7 {
            out.table_files += read(&cf, &format!("rocksdb.num-files-at-level{}", level));
        }
        first.get_or_insert(cf);
    }
    if let Some(cf) = first {
        out.block_cache_bytes = read(&cf, "rocksdb.block-cache-usage");
        out.block_cache_pinned_bytes = read(&cf, "rocksdb.block-cache-pinned-usage");
    }
    out
}
