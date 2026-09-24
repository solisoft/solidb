use dashmap::DashMap;
use rust_rocksdb::{BlockBasedOptions, Cache, ColumnFamilyDescriptor, DBCompressionType, Options};

use super::RocksDb as DB;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use super::collection::Collection;
use super::database::Database;
use super::pending_drops::{Claim, PendingCfDrops};
use crate::cluster::ClusterConfig;
use crate::error::{DbError, DbResult};
use crate::transaction::manager::TransactionManager;

/// Metadata column family name
pub(crate) const META_CF: &str = "_meta";

/// `_meta` key written by [`StorageEngine::flush_all_stats`] once every
/// collection's cached count is durable, and deleted by
/// [`StorageEngine::initialize`] the moment it is observed.
///
/// Its presence at startup means the previous run shut down gracefully, so the
/// on-disk counts can be trusted and the full recount skipped. Its absence —
/// a crash, a kill, or a first boot — triggers the recount. Deleting it during
/// startup is what makes a crash *after* a clean start still recount.
const CLEAN_SHUTDOWN_KEY: &str = "shutdown:clean";

/// Process-wide RocksDB memory/tuning profile.
///
/// Memory in RocksDB is dominated by per-CF structures (memtables, pinned
/// index/filter blocks). SoliDB maps one collection to one column family, so
/// on instances with thousands of collections (typically dev boxes that have
/// accumulated test/app databases) total RAM scales with the CF count. The
/// `dev` profile shrinks per-CF buffers and adds a *global* memtable budget so
/// idle CFs stop adding up. Prod keeps the throughput-oriented defaults.
#[derive(Clone, Copy, Debug)]
pub struct EngineProfile {
    /// Shared LRU block cache size (bytes).
    pub block_cache_bytes: usize,
    /// Per-CF memtable size (bytes).
    pub write_buffer_size: usize,
    /// Max memtables kept in memory per CF before flush.
    pub max_write_buffer_number: i32,
    /// Global cap on total memtable memory across ALL CFs (bytes).
    /// `None` leaves it unbounded (RocksDB default).
    pub db_write_buffer_size: Option<usize>,
    /// Background compaction/flush threads.
    pub max_background_jobs: i32,
    /// Open-file (table cache) limit; `-1` = unlimited.
    pub max_open_files: i32,
    /// Store index/filter blocks in the (bounded) block cache instead of
    /// pinning them per-CF. Caps index/filter RAM at the price of some reads.
    pub cache_index_and_filter_blocks: bool,
    /// Total WAL budget across all column families (bytes).
    ///
    /// Crossing it makes RocksDB flush **every** CF holding data in the oldest
    /// WAL (`DBImpl::SwitchWAL`), so a value small relative to the CF count is
    /// a stampede generator rather than a disk-usage cap: measured on a
    /// 963-CF instance at 50MB, 27904 of 27910 flushes had
    /// `flush_reason: "WAL Full"`, 97.7% of them writing under 4KB, in 39
    /// events averaging ~715 CFs each — and the WAL sat at 201MB regardless,
    /// because a WAL is only deletable once every CF that wrote to it has
    /// flushed. Keep it well above the working set and let
    /// `db_write_buffer_size` be the memory bound instead; that trigger
    /// flushes exactly one CF, the one with the oldest memtable.
    ///
    /// Never leave this at `0`: RocksDB then computes
    /// `4 × Σ(write_buffer_size × max_write_buffer_number)` over every CF.
    pub max_total_wal_size: usize,
    /// Initial memtable arena block (bytes).
    ///
    /// RocksDB's default is `min(1MB, write_buffer_size / 8)`, allocated per CF
    /// on first write. With thousands of near-empty column families that is
    /// most of the memtable footprint — ~963MB of arena for near-zero data on
    /// the instance above.
    pub arena_block_size: usize,
}

impl EngineProfile {
    /// Throughput-oriented defaults (production).
    pub const fn prod() -> Self {
        Self {
            block_cache_bytes: 512 * 1024 * 1024,
            write_buffer_size: 64 * 1024 * 1024,
            max_write_buffer_number: 3,
            // Bounded deliberately. Leaving this unset does not mean "no
            // flushing" — it means the WAL budget becomes the only trigger,
            // and that one flushes every CF at once. A global budget makes
            // the memory ceiling explicit and flushes one CF at a time.
            db_write_buffer_size: Some(512 * 1024 * 1024),
            max_background_jobs: 6,
            max_open_files: -1,
            cache_index_and_filter_blocks: false,
            max_total_wal_size: 2 * 1024 * 1024 * 1024,
            arena_block_size: 64 * 1024,
        }
    }

    /// Low-memory profile for dev boxes with many idle column families.
    pub const fn dev() -> Self {
        Self {
            block_cache_bytes: 128 * 1024 * 1024,
            write_buffer_size: 8 * 1024 * 1024,
            max_write_buffer_number: 2,
            db_write_buffer_size: Some(128 * 1024 * 1024),
            max_background_jobs: 2,
            max_open_files: 512,
            cache_index_and_filter_blocks: true,
            max_total_wal_size: 256 * 1024 * 1024,
            arena_block_size: 64 * 1024,
        }
    }
}

static PROFILE: OnceLock<EngineProfile> = OnceLock::new();

/// Select the process-wide engine profile. Must be called once, before the
/// first `StorageEngine` is constructed (i.e. before the block cache and any
/// CF options are built). Later calls are ignored.
pub fn set_engine_profile(profile: EngineProfile) {
    let _ = PROFILE.set(profile);
}

/// The active engine profile (defaults to `prod` if never set).
pub(crate) fn profile() -> EngineProfile {
    *PROFILE.get_or_init(EngineProfile::prod)
}

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
pub(crate) fn tuned_cf_options() -> Options {
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

/// The main storage engine backed by RocksDB
///
/// Uses lock-free reads - RocksDB's DB type is thread-safe for concurrent reads.
/// Writes are coordinated via RocksDB's internal MVCC and WriteBatch operations.
/// Only column family creation/deletion requires explicit locking.
pub struct StorageEngine {
    /// RocksDB instance - thread-safe for reads, internal locking for writes
    db: Arc<DB>,
    /// Lock for column family operations (create/delete)
    cf_lock: Arc<RwLock<()>>,
    /// Database path for reopening
    path: std::path::PathBuf,
    /// Cached collection handles (DashMap for lock-free concurrent access)
    collections: Arc<DashMap<String, Collection>>,
    /// Cached database handles (DashMap for lock-free concurrent access)
    databases: Arc<DashMap<String, Database>>,
    /// Cluster configuration (if running in cluster mode)
    cluster_config: Option<ClusterConfig>,
    /// Transaction manager, created on first use. Shared by every clone:
    /// each clone used to copy the `Option` it saw at clone time, so clones
    /// taken before initialisation each built their own manager — separate
    /// lock tables and active-transaction maps over the same data. A
    /// `OnceCell` also closes the check-then-set race in
    /// `initialize_transactions` (audit M7).
    transaction_manager: Arc<once_cell::sync::OnceCell<Arc<TransactionManager>>>,
    /// Column families scheduled for background drop (see `pending_drops`)
    pending_cf_drops: Arc<PendingCfDrops>,
    /// Cloned with the engine, and by nothing else. `Drop` runs once per
    /// clone, so this is how the *last* live handle recognises itself and
    /// takes on teardown work that must happen exactly once — joining the
    /// background CF droppers. `Arc<PendingCfDrops>`'s own count cannot serve:
    /// the dropper threads and every `Database` hold one too.
    liveness: Arc<()>,
}

impl Clone for StorageEngine {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            cf_lock: self.cf_lock.clone(),
            path: self.path.clone(),
            collections: self.collections.clone(),
            databases: self.databases.clone(),
            cluster_config: self.cluster_config.clone(),
            transaction_manager: Arc::clone(&self.transaction_manager),
            pending_cf_drops: self.pending_cf_drops.clone(),
            liveness: Arc::clone(&self.liveness),
        }
    }
}

impl std::fmt::Debug for StorageEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageEngine")
            .field("path", &self.path)
            .finish()
    }
}

impl StorageEngine {
    /// Create a new storage engine
    pub fn new<P: AsRef<Path>>(data_dir: P) -> DbResult<Self> {
        let path = data_dir.as_ref().to_path_buf();

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

        // Recycle LOG files instead of deleting
        opts.set_recycle_log_file_num(3);

        // Enable parallel memtable writes for better concurrency
        opts.set_enable_pipelined_write(true);

        // Get existing column families or create default
        let cf_names = match DB::list_cf(&opts, &path) {
            Ok(cfs) => cfs,
            Err(_) => vec!["default".to_string()],
        };

        // Ensure META_CF exists
        let mut cf_names: Vec<String> = cf_names.into_iter().collect();
        if !cf_names.contains(&META_CF.to_string()) {
            cf_names.push(META_CF.to_string());
        }

        // Create column family descriptors with optimized options
        // All column families inherit compression and performance settings
        let cf_descriptors: Vec<ColumnFamilyDescriptor> = cf_names
            .iter()
            .map(|name| ColumnFamilyDescriptor::new(name, tuned_cf_options()))
            .collect();

        // Open database with column families
        let db = DB::open_cf_descriptors(&opts, &path, cf_descriptors)
            .map_err(|e| DbError::InternalError(format!("Failed to open RocksDB: {}", e)))?;

        Ok(Self {
            db: Arc::new(db),
            cf_lock: Arc::new(RwLock::new(())),
            path,
            collections: Arc::new(DashMap::new()),
            databases: Arc::new(DashMap::new()),
            cluster_config: None,
            transaction_manager: Arc::new(once_cell::sync::OnceCell::new()),
            pending_cf_drops: PendingCfDrops::new(),
            liveness: Arc::new(()),
        })
    }

    /// Create a new storage engine with cluster configuration
    pub fn with_cluster_config<P: AsRef<Path>>(
        data_dir: P,
        config: ClusterConfig,
    ) -> DbResult<Self> {
        let mut engine = Self::new(data_dir)?;
        engine.cluster_config = Some(config);
        Ok(engine)
    }

    /// Get the cluster configuration
    pub fn cluster_config(&self) -> Option<&ClusterConfig> {
        self.cluster_config.as_ref()
    }

    /// Check if running in cluster mode
    pub fn is_cluster_mode(&self) -> bool {
        self.cluster_config
            .as_ref()
            .map(|c| c.is_cluster_mode())
            .unwrap_or(false)
    }

    /// Get node ID (returns "standalone" if not in cluster mode)
    pub fn node_id(&self) -> &str {
        self.cluster_config
            .as_ref()
            .map(|c| c.node_id.as_str())
            .unwrap_or("standalone")
    }

    /// Get the data directory path
    pub fn data_dir(&self) -> &str {
        self.path.to_str().unwrap_or("./data")
    }

    /// Create a consistent physical snapshot of the whole instance at `target`.
    ///
    /// This is the only physical backup mechanism: `solidb-dump` is a *logical*
    /// export, which is an order of magnitude larger on disk, far slower to
    /// restore, and offers no point-in-time consistency across collections.
    ///
    /// Scope is the entire RocksDB instance — every database and collection —
    /// because they all share one instance, with a column family per
    /// collection. There is no per-database checkpoint; use `solidb-dump` when
    /// you need a single database or cross-version portability.
    ///
    /// RocksDB hard-links SST files where the target is on the same
    /// filesystem, so the snapshot is near-instant and initially costs almost
    /// no extra space. It diverges from the live database as compaction
    /// rewrites files, so a checkpoint on the same volume is *not* protection
    /// against losing that volume — copy it off afterwards.
    ///
    /// `target` must not already exist; RocksDB refuses to write into an
    /// existing directory.
    pub fn create_checkpoint<P: AsRef<Path>>(&self, target: P) -> DbResult<()> {
        let target = target.as_ref();

        if target.exists() {
            return Err(DbError::BadRequest(format!(
                "checkpoint target '{}' already exists",
                target.display()
            )));
        }

        // Flush memtables first so the checkpoint reflects recent writes
        // without depending on WAL replay at restore time.
        if let Err(e) = self.db.flush() {
            tracing::warn!("checkpoint: flush before snapshot failed: {}", e);
        }

        let checkpoint = rust_rocksdb::checkpoint::Checkpoint::new(&*self.db).map_err(|e| {
            DbError::InternalError(format!("Failed to open checkpoint handle: {}", e))
        })?;

        checkpoint.create_checkpoint(target).map_err(|e| {
            DbError::InternalError(format!(
                "Failed to create checkpoint at '{}': {}",
                target.display(),
                e
            ))
        })?;

        tracing::info!("Created checkpoint at {}", target.display());
        Ok(())
    }

    /// Initialize the storage engine with default _system database
    pub fn initialize(&self) -> DbResult<()> {
        // Check if _system database exists
        let databases = self.list_databases();
        if !databases.contains(&"_system".to_string()) {
            // Create _system database
            self.create_database("_system".to_string())?;
        }

        // Ensure _config collection exists in _system (for cluster peer discovery)
        if let Ok(system_db) = self.get_database("_system") {
            if system_db.get_collection("_config").is_err() {
                let _ = system_db.create_collection("_config".to_string(), None);
            }
        }

        // Recalculate document counts for all collections, but only when the
        // previous run did not shut down cleanly — this walks every `doc:` key
        // of every collection, which on an instance with ~900 collections was
        // the bulk of a 22-33s startup (RocksDB's own open measured 1.8s).
        if self.take_clean_shutdown_marker() {
            tracing::info!("Clean shutdown detected — trusting persisted document counts");
        } else {
            self.recalculate_all_counts();
        }

        // Adopt any column family with no registry entry — created by a
        // pre-registry binary, or by a run that crashed between `create_cf`
        // and the entry write. This is what lets listing trust the registry.
        let pending = self.pending_cf_drops.clone();
        super::collection_registry::backfill(&self.db, |cf| pending.contains(cf));

        // Reclaim column families left by deleted collections once their
        // reuse grace expires (see `pending_drops::ensure_reaper`).
        PendingCfDrops::ensure_reaper(self.db.clone(), self.pending_cf_drops.clone());

        // Resume column-family drops interrupted by a previous shutdown/crash
        let resumed = self.pending_cf_drops.resume_from_meta(&self.db);
        if !resumed.is_empty() {
            tracing::info!(
                "Resuming {} interrupted column-family drops in the background",
                resumed.len()
            );
            PendingCfDrops::spawn_dropper(self.db.clone(), self.pending_cf_drops.clone(), resumed);
        }

        Ok(())
    }

    /// Read and clear the clean-shutdown marker.
    ///
    /// Returns whether it was set. Always clears it, so a crash later in this
    /// run leaves no stale marker for the next startup to trust.
    fn take_clean_shutdown_marker(&self) -> bool {
        let Some(meta_cf) = self.db.cf_handle(META_CF) else {
            return false;
        };
        let present = matches!(
            self.db.get_cf(&meta_cf, CLEAN_SHUTDOWN_KEY.as_bytes()),
            Ok(Some(_))
        );
        if present {
            if let Err(e) = self.db.delete_cf(&meta_cf, CLEAN_SHUTDOWN_KEY.as_bytes()) {
                // Leaving it set would let the *next* start skip the recount
                // after a crash, so treat a failed delete as unclean.
                tracing::warn!("Failed to clear the clean-shutdown marker: {}", e);
                return false;
            }
        }
        present
    }

    /// Record that every collection's cached count is durable on disk.
    fn set_clean_shutdown_marker(&self) {
        let Some(meta_cf) = self.db.cf_handle(META_CF) else {
            return;
        };
        if let Err(e) = self
            .db
            .put_cf(&meta_cf, CLEAN_SHUTDOWN_KEY.as_bytes(), b"1")
        {
            tracing::warn!("Failed to record a clean shutdown: {}", e);
        }
    }

    /// Recalculate document counts for all collections
    /// Called on startup to ensure counts are accurate after potential crashes
    pub fn recalculate_all_counts(&self) {
        let databases = self.list_databases();
        let mut total_collections = 0;

        for db_name in databases {
            if let Ok(database) = self.get_database(&db_name) {
                let collections = database.list_collections();
                for coll_name in collections {
                    if let Ok(collection) = database.system_collection(&coll_name) {
                        collection.recalculate_count();
                        total_collections += 1;
                    }
                }
            }
        }

        if total_collections > 0 {
            tracing::info!(
                "Recalculated document counts for {} collections",
                total_collections
            );
        }
    }

    /// Flush all collection stats and vector indexes to disk.
    /// Called on shutdown to ensure counts and the throttled vector-index
    /// persistence window are durable across a graceful restart.
    pub fn flush_all_stats(&self) {
        let databases = self.list_databases();

        for db_name in databases {
            if let Ok(database) = self.get_database(&db_name) {
                let collections = database.list_collections();
                for coll_name in collections {
                    if let Ok(collection) = database.system_collection(&coll_name) {
                        collection.flush_stats();
                        // Persist any vector-index changes that the per-write
                        // throttle deferred (see `persist_vector_indexes_throttled`).
                        collection.flush_vector_indexes();
                    }
                }
            }
        }

        // Also flush RocksDB
        let _ = self.flush();

        // Every cached count is now on disk, so the next startup can trust
        // them instead of walking every `doc:` key. Written *after* the flush
        // so the marker can never outrank the data it vouches for.
        self.set_clean_shutdown_marker();

        tracing::info!("Flushed all collection stats to disk");
    }

    // ==================== Database Operations ====================

    /// Create a new database
    pub fn create_database(&self, name: String) -> DbResult<()> {
        // Validate database name
        if name.is_empty() || name.contains(':') {
            return Err(DbError::InvalidDocument(
                "Invalid database name".to_string(),
            ));
        }

        // Check if database already exists by looking for any collection with this prefix
        let existing_dbs = self.list_databases();
        if existing_dbs.contains(&name) {
            return Err(DbError::CollectionAlreadyExists(format!(
                "Database '{}' already exists",
                name
            )));
        }

        // Store database metadata (RocksDB is thread-safe for writes)
        let meta_cf = self.db.cf_handle(META_CF).expect("META_CF should exist");
        let db_key = format!("db:{}", name);
        self.db
            .put_cf(&meta_cf, db_key.as_bytes(), b"1")
            .map_err(|e| DbError::InternalError(format!("Failed to create database: {}", e)))?;

        Ok(())
    }

    /// Delete a database and all its collections.
    ///
    /// The database is removed from metadata immediately; the per-collection
    /// column-family drops run on a background thread. Each `drop_cf`
    /// rewrites + fsyncs the entire OPTIONS file (one section per CF), so on
    /// an instance with many CFs dropping a database inline would block the
    /// request for `collections × hundreds-of-ms` (measured: 18s for 25
    /// collections at ~1800 CFs). See `storage::pending_drops`.
    pub fn delete_database(&self, name: &str) -> DbResult<()> {
        // Prevent deletion of _system database
        if name == "_system" {
            return Err(DbError::InvalidDocument(
                "Cannot delete _system database".to_string(),
            ));
        }

        // Ensure the database exists
        if !self.list_databases().contains(&name.to_string()) {
            return Err(DbError::CollectionNotFound(format!(
                "Database '{}' not found",
                name
            )));
        }

        // Every CF belonging to this database (document + columnar), minus
        // any already scheduled by a previous drop of the same name
        let prefix = format!("{}:", name);
        let doomed: Vec<String> = self
            .db
            .cf_names()
            .into_iter()
            .filter(|cf| cf.starts_with(&prefix) && !self.pending_cf_drops.contains(cf))
            .collect();

        // Atomically delete the `db:{name}` metadata key and persist a
        // `pending_drop:` marker per CF, then drop the CFs in the background.
        // Markers survive a crash and are resumed by `initialize`.
        let db_key = format!("db:{}", name);
        self.pending_cf_drops.schedule(&self.db, &db_key, &doomed)?;

        // Remove from cache
        self.databases.remove(name);

        // Purge stale Collection handles for this database. A cached handle
        // resolves its CF by name at use time, so once the background dropper
        // removes the CF any holder of the cached handle panics ("Column
        // family should exist") on its next operation — observed killing
        // in-flight connections when a database is dropped and immediately
        // recreated (e.g. test suites, CREATE MATERIALIZED VIEW on the
        // recreated db touching the doomed `_views` CF).
        self.collections
            .retain(|cf_name, _| !cf_name.starts_with(&prefix));

        PendingCfDrops::spawn_dropper(self.db.clone(), self.pending_cf_drops.clone(), doomed);

        Ok(())
    }

    /// List all databases
    pub fn list_databases(&self) -> Vec<String> {
        // Lock-free read - RocksDB is thread-safe
        let meta_cf = match self.db.cf_handle(META_CF) {
            Some(cf) => cf,
            None => return vec![],
        };

        let prefix = b"db:";
        let iter = self.db.prefix_iterator_cf(&meta_cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, _)| {
                let key_str = String::from_utf8(key.to_vec()).ok()?;
                key_str.strip_prefix("db:").map(|s| s.to_string())
            })
        })
        .collect()
    }

    /// Get a database handle (cached for consistent collection counters)
    pub fn get_database(&self, name: &str) -> DbResult<Database> {
        // Check cache first (DashMap allows concurrent read without locking)
        if let Some(database) = self.databases.get(name) {
            return Ok(database.clone());
        }

        // Verify database exists
        let databases = self.list_databases();
        if !databases.contains(&name.to_string()) {
            return Err(DbError::CollectionNotFound(format!(
                "Database '{}' not found",
                name
            )));
        }

        // Create and cache the database
        let database = Database::new(
            name.to_string(),
            self.db.clone(),
            self.pending_cf_drops.clone(),
        )
        .with_engine_cache(self.collections.clone());
        self.databases.insert(name.to_string(), database.clone());

        Ok(database)
    }

    // ==================== Legacy Collection Operations (for backward compatibility) ====================

    /// Create a new collection (column family)
    pub fn create_collection(&self, name: String, collection_type: Option<String>) -> DbResult<()> {
        // Default to "document" if not specified
        let type_ = collection_type.unwrap_or_else(|| "document".to_string());

        // Create the column family - requires exclusive lock
        let opts = tuned_cf_options();
        {
            let _cf_guard = self.cf_lock.write().unwrap();

            // The CF may be a leftover from a dropped database still awaiting
            // its background drop — claim it and recreate fresh instead of
            // failing with "already exists" or, worse, leaving the new
            // collection on a doomed CF the background dropper then removes.
            // (Mirrors Database::create_collection.)
            let mut reused = false;
            match self.pending_cf_drops.claim_for_recreate(&name) {
                Claim::Claimed => {
                    // Wipe and reuse rather than drop and recreate: the pair
                    // costs two full OPTIONS rewrites to end up where it
                    // started. Falls back to the drop if the wipe fails,
                    // rather than handing out a CF that may still hold the
                    // previous incarnation's data.
                    if self.db.cf_handle(&name).is_some() {
                        match super::cf_ops::wipe_cf(&self.db, &name) {
                            Ok(()) => {
                                reused = true;
                                super::cf_ops::record_reuse();
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Reusing column family '{}' failed ({}); dropping it instead",
                                    name,
                                    e
                                );
                                if let Err(e) = super::cf_ops::timed(|| self.db.drop_cf(&name)) {
                                    self.pending_cf_drops.release_claim(&name);
                                    return Err(DbError::InternalError(format!(
                                        "Failed to reclaim pending collection: {}",
                                        e
                                    )));
                                }
                            }
                        }
                    }
                    self.pending_cf_drops.complete(&self.db, &name);
                }
                Claim::InProgress => {
                    // The background dropper is dropping this exact CF right
                    // now — wait for it to finish, then create fresh below.
                    self.pending_cf_drops
                        .wait_until_dropped(&name, std::time::Duration::from_secs(30))?;
                }
                Claim::NotPending => {
                    // Check inside lock to avoid TOCTOU race when multiple
                    // threads try to create the same collection concurrently
                    if self.db.cf_handle(&name).is_some() {
                        return Err(DbError::CollectionAlreadyExists(name));
                    }
                }
            }

            if !reused {
                // MultiThreaded mode: create_cf takes &self and synchronizes internally
                super::cf_ops::timed(|| self.db.create_cf(&name, &opts)).map_err(|e| {
                    DbError::InternalError(format!("Failed to create collection: {}", e))
                })?;
            }
        }

        // A cached handle from the pre-drop incarnation of this CF must not
        // shadow the fresh one — in this cache or the owning database's.
        self.collections.remove(&name);
        self.evict_database_cached_collection(&name);
        super::collection::index_meta::invalidate_index_meta(&self.db, &name);

        // Persist collection type (lock-free, thread-safe)
        if let Some(cf) = self.db.cf_handle(&name) {
            self.db
                .put_cf(&cf, "_stats:type".as_bytes(), type_.as_bytes())
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to set collection type: {}", e))
                })?;
        }

        // Register last, so an entry never outruns its column family.
        if let Err(e) = super::collection_registry::record(&self.db, &name, &type_) {
            tracing::warn!(
                "Collection '{}' created but not registered ({}); \
                 the next startup will adopt it",
                name,
                e
            );
        }

        Ok(())
    }

    /// Get a collection by a name that came from a caller (legacy method -
    /// checks both database-prefixed and plain names).
    ///
    /// Refuses the credential collections, for the same reason as
    /// [`Database::get_collection`] (SEC-176). This level needs its own guard:
    /// the transactional handlers resolve caller-supplied collection names
    /// here rather than through a `Database`, so guarding only the `Database`
    /// accessor left `/transaction/{tx}/document/_env/...` open — and because
    /// an unqualified name falls back to `_system:{name}` below, that reached
    /// the *instance-wide* credentials from any database.
    pub fn get_collection(&self, name: &str) -> DbResult<Collection> {
        if crate::storage::is_protected_collection(name) {
            return Err(crate::storage::protected_collection_error(name));
        }
        self.system_collection(name)
    }

    /// Unrestricted variant of [`Self::get_collection`], for server-side code
    /// applying already-authorized work (e.g. committing a transaction's
    /// operations). Never pass a caller-supplied name to this.
    pub fn system_collection(&self, name: &str) -> DbResult<Collection> {
        // A CF scheduled for background drop must read as already deleted —
        // serving it (cached or fresh) hands out a handle whose CF can vanish
        // mid-operation.
        if self.pending_cf_drops.contains(name) {
            self.collections.remove(name);
            return Err(DbError::CollectionNotFound(name.to_string()));
        }

        // Check cache first (DashMap allows concurrent read without locking)
        if let Some(collection) = self.collections.get(name) {
            return Ok(collection.clone());
        }

        // First, try the exact name (for backward compatibility or direct access)
        let actual_name = if self.db.cf_handle(name).is_some() {
            name.to_string()
        } else {
            // If not found, try prefixing with _system database
            let system_name = format!("_system:{}", name);
            if self.pending_cf_drops.contains(&system_name) {
                self.collections.remove(&system_name);
                return Err(DbError::CollectionNotFound(name.to_string()));
            }
            if self.db.cf_handle(&system_name).is_some() {
                system_name
            } else {
                // Not found in either format
                return Err(DbError::CollectionNotFound(name.to_string()));
            }
        };

        // Resolve through the owning `Database` so there is exactly one
        // `Collection` — and therefore one `change_sender` — per (database,
        // collection). This used to mint a second instance with its own
        // broadcast channel, so a write arriving through the engine (the
        // `/transaction/{tx}/document/...` endpoints) fired into a ring that no
        // changefeed subscriber was listening to.
        //
        // A CF can outlive its `db:` metadata key (an interrupted drop), and the
        // engine path has to keep serving those; fall back to a direct handle
        // rather than refusing, which is what this did before.
        let collection = match actual_name.split_once(':') {
            Some((db_name, coll_name)) => self
                .get_database(db_name)
                .and_then(|db| db.system_collection(coll_name))
                .unwrap_or_else(|_| Collection::new(actual_name.clone(), self.db.clone())),
            None => Collection::new(actual_name.clone(), self.db.clone()),
        };

        self.collections
            .insert(name.to_string(), collection.clone());
        if actual_name != name {
            self.collections.insert(actual_name, collection.clone());
        }

        Ok(collection)
    }

    /// Delete a collection
    pub fn delete_collection(&self, name: &str) -> DbResult<()> {
        if self.db.cf_handle(name).is_none() {
            return Err(DbError::CollectionNotFound(name.to_string()));
        }

        // MultiThreaded mode: drop_cf takes &self and synchronizes internally
        super::cf_ops::timed(|| self.db.drop_cf(name))
            .map_err(|e| DbError::InternalError(format!("Failed to delete collection: {}", e)))?;

        if let Err(e) = super::collection_registry::forget(&self.db, name) {
            tracing::warn!("Failed to deregister collection '{}': {}", name, e);
        }

        // Drop the stale cached handle so a later same-name create starts fresh.
        self.collections.remove(name);
        self.evict_database_cached_collection(name);
        super::collection::index_meta::invalidate_index_meta(&self.db, name);

        Ok(())
    }

    /// Evict `db:coll` from the owning `Database`'s handle cache, if that
    /// database is loaded (audit D8: the two caches share instances).
    fn evict_database_cached_collection(&self, cf_name: &str) {
        if let Some((db_name, coll_name)) = cf_name.split_once(':') {
            if let Some(database) = self.databases.get(db_name) {
                database.evict_cached_collection(coll_name);
            }
        }
    }

    /// List all collection names
    pub fn list_collections(&self) -> Vec<String> {
        // Use the live in-memory CF list — DB::list_cf would re-read the
        // MANIFEST from disk on every call
        self.db
            .cf_names()
            .into_iter()
            .filter(|name| name != "default" && name != META_CF)
            .collect()
    }

    /// Every database's collections, grouped, from a single pass over the
    /// column-family list.
    ///
    /// [`Database::list_collections`] scans *all* column families to find the
    /// ones carrying its prefix, so calling it once per database is O(dbs ×
    /// cfs) — and `cf_names()` clones every name on each call. A sweep over 89
    /// databases holding 1718 collections allocated ~153k strings per pass;
    /// this does 1718. Callers that walk the whole instance (the cluster stats
    /// collector, the heartbeat, the status endpoint) should use this instead.
    ///
    /// Column families awaiting a background drop are omitted, matching
    /// [`Database::list_collections`].
    ///
    /// Keys come from the column-family names, *not* from the database
    /// registry: a CF can outlive its `db:` entry when a drop is interrupted.
    /// Callers enumerating databases should drive from [`Self::list_databases`]
    /// and look the group up here.
    pub fn collections_grouped(&self) -> HashMap<String, Vec<String>> {
        let mut grouped: HashMap<String, Vec<String>> = HashMap::new();

        // From the `_meta` registry rather than `DB::cf_names()`, which clones
        // every column-family name in the instance and contends with the CF
        // map's write lock (held across each OPTIONS rewrite). Falls back to
        // the column-family map when there is no `_meta` to consult.
        let registered = if super::collection_registry::available(&self.db) {
            super::collection_registry::list_all(&self.db)
        } else {
            self.db
                .cf_names()
                .into_iter()
                .filter(|cf| cf != "default" && cf != META_CF)
                .collect()
        };

        for cf_name in registered {
            if self.pending_cf_drops.contains(&cf_name) {
                continue;
            }
            // Collection CFs are named "<database>:<collection>"; anything
            // without the separator is not one.
            let Some((db_name, coll_name)) = cf_name.split_once(':') else {
                continue;
            };
            grouped
                .entry(db_name.to_string())
                .or_default()
                .push(coll_name.to_string());
        }

        grouped
    }

    /// Per-component memory attribution, for the `/metrics` endpoint.
    ///
    /// Exists because a 613-collection instance was OOM-killed at 21.7 GB RSS
    /// while holding 6.3 GB of data, and the arithmetic that *looks* like it
    /// explains that (613 collections x the 64 MB per-CF write buffer) is not
    /// evidence — several other consumers are unbounded too. These are the
    /// numbers that say which one actually grew.
    ///
    /// Costs one FFI property read per column family per counter, so this is
    /// computed on demand at scrape time and never on a timer.
    pub fn memory_breakdown(&self) -> MemoryBreakdown {
        let mut out = MemoryBreakdown::default();

        let cf_names: Vec<String> = self
            .db
            .cf_names()
            .into_iter()
            .filter(|n| !self.pending_cf_drops.contains(n))
            .collect();
        out.column_families = cf_names.len() as u64;

        // `AsColumnFamilyRef` is implemented for `&Arc<BoundColumnFamily>`,
        // which is what `cf_handle` hands back — matching the existing reads in
        // `collection::core`.
        let read = |cf: &Arc<rust_rocksdb::BoundColumnFamily<'_>>, prop: &str| -> u64 {
            self.db
                .property_int_value_cf(cf, prop)
                .ok()
                .flatten()
                .unwrap_or(0)
        };

        for name in &cf_names {
            let Some(cf) = self.db.cf_handle(name) else {
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
                out.sst_files += read(&cf, &format!("rocksdb.num-files-at-level{}", level));
            }
        }

        // The block cache is one shared LRU for every CF
        // (`shared_block_cache`), and these two properties report that shared
        // instance. Reading them per CF and summing would multiply it by the
        // column-family count — take them from one handle only.
        if let Some(cf) = cf_names.first().and_then(|n| self.db.cf_handle(n)) {
            out.block_cache_bytes = read(&cf, "rocksdb.block-cache-usage");
            out.block_cache_pinned_bytes = read(&cf, "rocksdb.block-cache-pinned-usage");
        }

        // The handles live in each `Database`'s own cache, not in the engine's
        // (`self.collections` is the legacy per-engine one and holds almost
        // nothing). Summing the per-database maps is what actually counts the
        // 600-odd live handles, each of which carries a broadcast ring.
        out.cached_collection_handles = self
            .databases
            .iter()
            .map(|entry| entry.value().cached_collection_count() as u64)
            .sum::<u64>()
            + self.collections.len() as u64;

        out
    }

    /// Save a collection - no-op with RocksDB (auto-persisted)
    pub fn save_collection(&self, _name: &str) -> DbResult<()> {
        // RocksDB automatically persists data, nothing to do
        Ok(())
    }

    /// Flush all pending writes to disk
    pub fn flush(&self) -> DbResult<()> {
        self.db
            .flush()
            .map_err(|e| DbError::InternalError(format!("Failed to flush: {}", e)))?;
        Ok(())
    }

    // ==================== Transaction Operations ====================

    /// Initialize transaction manager (call once on startup if transactions are needed)
    ///
    /// Runs at most once per engine (and its clones): concurrent first
    /// callers block on the cell rather than each replaying the WAL.
    pub fn initialize_transactions(&self) -> DbResult<()> {
        self.transaction_manager
            .get_or_try_init(|| -> DbResult<Arc<TransactionManager>> {
                let wal_path = self.path.join("transaction.wal");

                // Recover any committed transactions from WAL BEFORE creating
                // the manager.
                let recovered = self.recover_transactions()?;

                let manager = TransactionManager::new(wal_path)?;

                // Audit M7: the log holds nothing that is not applied now, so
                // empty it instead of re-reading an ever-growing file at every
                // startup. Kept when recovery skipped something, so it stays
                // available for inspection.
                if recovered {
                    if let Err(e) = manager.checkpoint() {
                        tracing::warn!("Failed to truncate transaction WAL: {}", e);
                    }
                }

                tracing::info!("Transaction manager initialized");
                Ok(Arc::new(manager))
            })
            .map(|_| ())
    }

    /// Get transaction manager (initializes if needed)
    pub fn transaction_manager(&self) -> DbResult<Arc<TransactionManager>> {
        if let Some(manager) = self.transaction_manager.get() {
            return Ok(manager.clone());
        }
        self.initialize_transactions()?;
        self.transaction_manager.get().cloned().ok_or_else(|| {
            DbError::InternalError("Transaction manager not initialized".to_string())
        })
    }

    /// The manager, only if something already initialized it.
    fn initialized_transaction_manager(&self) -> DbResult<Arc<TransactionManager>> {
        self.transaction_manager.get().cloned().ok_or_else(|| {
            DbError::InternalError("Transaction manager not initialized".to_string())
        })
    }

    /// Recover committed transactions from WAL (called on startup).
    ///
    /// Returns whether every committed transaction in the log was applied.
    /// Since operations stopped being logged, a log written by this version
    /// holds none; replay is kept for logs left by older ones.
    fn recover_transactions(&self) -> DbResult<bool> {
        use crate::transaction::wal::WalReader;

        let wal_path = self.path.join("transaction.wal");
        if !wal_path.exists() {
            return Ok(true); // No WAL to recover
        }

        let reader = WalReader::new(&wal_path);
        let committed_txs = reader.replay()?;

        let with_ops: Vec<_> = committed_txs
            .into_iter()
            .filter(|tx| !tx.operations.is_empty())
            .collect();
        if with_ops.is_empty() {
            return Ok(true);
        }

        tracing::info!(
            "Recovering {} committed transactions from WAL",
            with_ops.len()
        );

        let mut complete = true;
        for tx in with_ops {
            // A conflict here most likely means the transaction was already
            // applied before the restart; skip it rather than refuse to start.
            if let Err(e) = self.apply_operations_atomically(&tx.operations, true) {
                tracing::warn!("Skipping transaction {} during WAL recovery: {}", tx.id, e);
                complete = false;
            }
        }

        tracing::info!("Transaction recovery complete");
        Ok(complete)
    }

    /// Stage every operation, across all collections, into ONE `WriteBatch`
    /// and write it. All collections are column families of the same RocksDB
    /// instance, so the batch is atomic across them: either every document,
    /// index entry and version record lands, or none does. In-memory effects
    /// (vector indexes, counts, change events) run only after the write.
    fn apply_operations_atomically(
        &self,
        operations: &[crate::transaction::Operation],
        sync: bool,
    ) -> DbResult<()> {
        // Group by collection, keeping each collection's operations in order.
        let mut order: Vec<String> = Vec::new();
        let mut ops_by_collection: HashMap<String, Vec<crate::transaction::Operation>> =
            HashMap::new();
        for op in operations {
            let coll_name = format!("{}:{}", op.database(), op.collection());
            let entry = ops_by_collection.entry(coll_name.clone()).or_default();
            if entry.is_empty() {
                order.push(coll_name);
            }
            entry.push(op.clone());
        }

        // Collections are locked in name order: a commit is the only writer
        // that holds write stripes of several collections at once, and a
        // fixed order keeps two commits from deadlocking on them.
        order.sort();

        let mut batch = rust_rocksdb::WriteBatch::default();
        let mut staged = Vec::with_capacity(order.len());
        let mut guards = Vec::new();
        for coll_name in order {
            let ops = ops_by_collection.remove(&coll_name).unwrap_or_default();
            let collection = self.system_collection(&coll_name)?;
            guards.extend(collection.lock_for_transaction(&ops));
            let plan = collection.stage_transaction_operations(&ops, &mut batch)?;
            staged.push((collection, plan));
        }

        let mut write_opts = rust_rocksdb::WriteOptions::default();
        write_opts.set_sync(sync);
        self.db.write_opt(&batch, &write_opts).map_err(|e| {
            DbError::InternalError(format!("Failed to commit transaction batch: {}", e))
        })?;
        drop(guards);

        for (collection, plan) in staged {
            collection.finish_transaction_operations(plan);
        }
        Ok(())
    }

    /// Commit a transaction by applying all operations atomically.
    ///
    /// Audit D2: this used to write one batch per collection and only then
    /// validate, so a failed validation or a failure in a later collection
    /// left earlier writes committed, and the transaction (with its locks)
    /// lingered until the expiry reaper. Now: validate, stage everything into
    /// one batch, write once. Any failure writes nothing, and the transaction
    /// is removed and its locks released before the error is returned.
    pub fn commit_transaction(&self, tx_id: crate::transaction::TransactionId) -> DbResult<()> {
        let manager = self.initialized_transaction_manager()?;

        // Freezes the transaction and validates it; aborts it on failure.
        let (operations, sync) = manager.prepare_commit(tx_id)?;

        // Staging re-reads every touched document while this transaction
        // still holds its exclusive locks, so the conflict checks see the
        // state the write will land on.
        if let Err(e) = self.apply_operations_atomically(&operations, sync) {
            manager.abort(tx_id);
            return Err(e);
        }

        manager.finish_commit(tx_id)
    }

    /// Rollback a transaction (operations already in WAL as aborted)
    pub fn rollback_transaction(&self, tx_id: crate::transaction::TransactionId) -> DbResult<()> {
        let manager = self.initialized_transaction_manager()?;

        // Just mark as aborted - operations were never applied
        manager.rollback(tx_id)?;

        Ok(())
    }
}

/// Where a SoliDB process's RocksDB memory actually is.
///
/// One field per consumer that can grow without a ceiling, so a growth event
/// can be attributed instead of guessed at. See
/// [`StorageEngine::memory_breakdown`].
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct MemoryBreakdown {
    /// Live memtables across every column family.
    pub memtable_bytes: u64,
    /// Live plus immutable memtables; the gap over `memtable_bytes` is flush
    /// backlog. Capped only by `--memtable-budget`, which is unset in prod.
    pub memtable_total_bytes: u64,
    /// Index and filter blocks pinned per open SST, *outside* the block cache
    /// unless `--bounded-index-cache` is set. Never evicted.
    pub table_readers_bytes: u64,
    /// The single shared block cache (`--block-cache`, 512 MB in prod).
    pub block_cache_bytes: u64,
    /// The part of that cache that cannot be evicted.
    pub block_cache_pinned_bytes: u64,
    /// SST files across all levels of all column families.
    pub sst_files: u64,
    /// Column families, i.e. collections plus `default` and `_meta`.
    pub column_families: u64,
    /// `Collection` handles held in the engine's unbounded cache.
    pub cached_collection_handles: u64,
}

impl Drop for StorageEngine {
    fn drop(&mut self) {
        // Only the last live handle tears down. `StorageEngine` is Clone and
        // this runs for every clone, so without the check a clone going out of
        // scope mid-request would block on the join below.
        let last = Arc::strong_count(&self.liveness) == 1;

        // Clear collections and databases before RocksDB is dropped
        // This ensures proper cleanup order and avoids pthread mutex issues
        self.collections.clear();
        self.databases.clear();

        if last {
            // Before the flush and before this handle's `Arc<DB>` goes away:
            // a background dropper still inside `drop_cf` races the static
            // destructors that free RocksDB's global option-type registry, and
            // the process aborts with SIGSEGV or `std::bad_alloc` *after* all
            // its work reported success. This is what made
            // `rbac_admin_endpoints_tests` fail at process exit with every test
            // green, and it was the same race on the server's shutdown path.
            self.pending_cf_drops.join_droppers();
        }

        // Flush RocksDB before drop (DB is thread-safe, direct access is safe)
        let _ = self.db.flush();
    }
}
