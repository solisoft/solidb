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
}

impl EngineProfile {
    /// Throughput-oriented defaults (production).
    pub const fn prod() -> Self {
        Self {
            block_cache_bytes: 512 * 1024 * 1024,
            write_buffer_size: 64 * 1024 * 1024,
            max_write_buffer_number: 3,
            db_write_buffer_size: None,
            max_background_jobs: 6,
            max_open_files: -1,
            cache_index_and_filter_blocks: false,
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
    /// Transaction manager (optionally initialized, uses RwLock for interior mutability)
    transaction_manager: RwLock<Option<Arc<TransactionManager>>>,
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
            transaction_manager: RwLock::new(
                self.transaction_manager.read().ok().and_then(|t| t.clone()),
            ),
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

        // Limit WAL file size to prevent unbounded disk growth
        // Max total WAL size across all column families: 50MB
        opts.set_max_total_wal_size(50 * 1024 * 1024);

        // Keep fewer LOG files (RocksDB info logs, not WALs)
        opts.set_keep_log_file_num(5);

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
            transaction_manager: RwLock::new(None),
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

        // Recalculate document counts for all collections
        // This ensures counts are accurate after crashes or unclean shutdowns
        self.recalculate_all_counts();

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
        );
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
            match self.pending_cf_drops.claim_for_recreate(&name) {
                Claim::Claimed => {
                    if self.db.cf_handle(&name).is_some() {
                        if let Err(e) = super::cf_ops::timed(|| self.db.drop_cf(&name)) {
                            self.pending_cf_drops.release_claim(&name);
                            return Err(DbError::InternalError(format!(
                                "Failed to reclaim pending collection: {}",
                                e
                            )));
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

            // MultiThreaded mode: create_cf takes &self and synchronizes internally
            super::cf_ops::timed(|| self.db.create_cf(&name, &opts)).map_err(|e| {
                DbError::InternalError(format!("Failed to create collection: {}", e))
            })?;
        }

        // A cached handle from the pre-drop incarnation of this CF must not
        // shadow the fresh one.
        self.collections.remove(&name);
        super::collection::index_meta::invalidate_index_meta(&self.db, &name);

        // Persist collection type (lock-free, thread-safe)
        if let Some(cf) = self.db.cf_handle(&name) {
            self.db
                .put_cf(&cf, "_stats:type".as_bytes(), type_.as_bytes())
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to set collection type: {}", e))
                })?;
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

        // Create and cache the collection
        let collection = Collection::new(actual_name.clone(), self.db.clone());
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

        // Drop the stale cached handle so a later same-name create starts fresh.
        self.collections.remove(name);
        super::collection::index_meta::invalidate_index_meta(&self.db, name);

        Ok(())
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

        for cf_name in self.db.cf_names() {
            if cf_name == "default" || cf_name == META_CF {
                continue;
            }
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
    pub fn initialize_transactions(&self) -> DbResult<()> {
        // Check if already initialized (read lock first)
        {
            if let Ok(tx_mgr) = self.transaction_manager.read() {
                if tx_mgr.is_some() {
                    return Ok(()); // Already initialized
                }
            }
        }

        let wal_path = self.path.join("transaction.wal");

        // Recover any committed transactions from WAL BEFORE creating manager
        // This ensures we don't double-apply on restart
        self.recover_transactions()?;

        let manager = TransactionManager::new(wal_path)?;

        // Now acquire write lock to store manager
        {
            if let Ok(mut tx_mgr) = self.transaction_manager.write() {
                *tx_mgr = Some(Arc::new(manager));
            }
        }

        tracing::info!("Transaction manager initialized");
        Ok(())
    }

    /// Get transaction manager (initializes if needed)
    pub fn transaction_manager(&self) -> DbResult<Arc<TransactionManager>> {
        // Try to read first
        {
            if let Ok(tx_mgr) = self.transaction_manager.read() {
                if let Some(ref manager) = *tx_mgr {
                    return Ok(manager.clone());
                }
            }
        }

        // Not initialized, so initialize it
        self.initialize_transactions()?;

        // Read again after initialization
        if let Ok(tx_mgr) = self.transaction_manager.read() {
            if let Some(manager) = tx_mgr.as_ref() {
                return Ok(manager.clone());
            }
        }

        Err(DbError::InternalError(
            "Transaction manager not initialized".to_string(),
        ))
    }

    /// Recover committed transactions from WAL (called on startup)
    fn recover_transactions(&self) -> DbResult<()> {
        use crate::transaction::wal::WalReader;

        let wal_path = self.path.join("transaction.wal");
        if !wal_path.exists() {
            return Ok(()); // No WAL to recover
        }

        let reader = WalReader::new(&wal_path);
        let committed_txs = reader.replay()?;

        if committed_txs.is_empty() {
            return Ok(());
        }

        tracing::info!(
            "Recovering {} committed transactions from WAL",
            committed_txs.len()
        );

        // Apply each committed transaction
        for tx in committed_txs {
            // Group operations by collection
            let mut ops_by_collection: HashMap<String, Vec<crate::transaction::Operation>> =
                HashMap::new();

            for op in tx.operations {
                let coll_name = format!("{}:{}", op.database(), op.collection());
                ops_by_collection.entry(coll_name).or_default().push(op);
            }

            // Apply operations for each collection
            for (coll_name, ops) in ops_by_collection {
                if let Ok(collection) = self.system_collection(&coll_name) {
                    collection.apply_transaction_operations(ops)?;
                } else {
                    tracing::warn!(
                        "Collection {} not found during WAL recovery, skipping",
                        coll_name
                    );
                }
            }
        }

        tracing::info!("Transaction recovery complete");
        Ok(())
    }

    /// Commit a transaction by applying all operations atomically
    pub fn commit_transaction(&self, tx_id: crate::transaction::TransactionId) -> DbResult<()> {
        let manager = {
            if let Ok(tx_mgr) = self.transaction_manager.read() {
                if let Some(mgr) = tx_mgr.as_ref() {
                    mgr.clone()
                } else {
                    return Err(DbError::InternalError(
                        "Transaction manager not initialized".to_string(),
                    ));
                }
            } else {
                return Err(DbError::InternalError(
                    "Transaction manager lock failed".to_string(),
                ));
            }
        };

        // Get transaction
        let tx_arc = manager.get(tx_id)?;
        let operations = {
            let tx = tx_arc.read().expect("Transaction lock poisoned");
            tx.operations.clone()
        };

        // Group operations by collection
        let mut ops_by_collection: HashMap<String, Vec<crate::transaction::Operation>> =
            HashMap::new();

        for op in operations {
            let coll_name = format!("{}:{}", op.database(), op.collection());
            ops_by_collection.entry(coll_name).or_default().push(op);
        }

        // Apply operations for each collection
        for (coll_name, ops) in ops_by_collection {
            let collection = self.system_collection(&coll_name)?;
            collection.apply_transaction_operations(ops)?;
        }

        // Mark transaction as committed in manager
        manager.commit(tx_id)?;

        Ok(())
    }

    /// Rollback a transaction (operations already in WAL as aborted)
    pub fn rollback_transaction(&self, tx_id: crate::transaction::TransactionId) -> DbResult<()> {
        let manager = {
            if let Ok(tx_mgr) = self.transaction_manager.read() {
                if let Some(mgr) = tx_mgr.as_ref() {
                    mgr.clone()
                } else {
                    return Err(DbError::InternalError(
                        "Transaction manager not initialized".to_string(),
                    ));
                }
            } else {
                return Err(DbError::InternalError(
                    "Transaction manager lock failed".to_string(),
                ));
            }
        };

        // Just mark as aborted - operations were never applied
        manager.rollback(tx_id)?;

        Ok(())
    }
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
        if let Ok(mut tm) = self.transaction_manager.write() {
            *tm = None;
        }

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
