use super::types::IsolationLevel;
use serde::{Deserialize, Serialize};
use serde_json::Value; // Added this import

/// Commands that can be sent to the server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Authenticate with the server
    Auth {
        database: String,
        username: String,
        password: String,
    },

    /// Ping the server (keep-alive)
    Ping,

    // ==================== Database Operations ====================
    /// List all databases
    ListDatabases,

    /// Create a new database
    CreateDatabase { name: String },

    /// Delete a database
    DeleteDatabase { name: String },

    // ==================== Collection Operations ====================
    /// List collections in a database
    ListCollections { database: String },

    /// Create a new collection
    CreateCollection {
        database: String,
        name: String,
        #[serde(rename = "type")]
        collection_type: Option<String>,
    },

    /// Delete a collection
    DeleteCollection { database: String, name: String },

    /// Get collection statistics
    CollectionStats { database: String, name: String },

    // ==================== Document Operations ====================
    /// Get a document by key
    Get {
        database: String,
        collection: String,
        key: String,
    },

    /// Insert a new document
    Insert {
        database: String,
        collection: String,
        key: Option<String>,
        document: Value,
    },

    /// Update a document
    Update {
        database: String,
        collection: String,
        key: String,
        document: Value,
        /// If true, merge with existing document (PATCH-like)
        #[serde(default)]
        merge: bool,
    },

    /// Delete a document
    Delete {
        database: String,
        collection: String,
        key: String,
    },

    /// List documents with pagination
    List {
        database: String,
        collection: String,
        limit: Option<usize>,
        offset: Option<usize>,
    },

    // ==================== Query Operations ====================
    /// Execute a SDBQL query
    Query {
        database: String,
        sdbql: String,
        bind_vars: Option<std::collections::HashMap<String, Value>>,
    },

    /// Explain a SDBQL query
    Explain {
        database: String,
        sdbql: String,
        bind_vars: Option<std::collections::HashMap<String, Value>>,
    },

    // ==================== Index Operations ====================
    /// Create an index
    CreateIndex {
        database: String,
        collection: String,
        name: String,
        fields: Vec<String>,
        #[serde(default)]
        unique: bool,
        #[serde(default)]
        sparse: bool,
    },

    /// Delete an index
    DeleteIndex {
        database: String,
        collection: String,
        name: String,
    },

    /// List indexes
    ListIndexes { database: String, collection: String },

    // ==================== Transaction Operations ====================
    /// Begin a transaction
    BeginTransaction {
        database: String,
        #[serde(default)]
        isolation_level: IsolationLevel,
    },

    /// Commit a transaction
    CommitTransaction { tx_id: String },

    /// Rollback a transaction
    RollbackTransaction { tx_id: String },

    /// Execute a command within a transaction
    TransactionCommand {
        tx_id: String,
        command: Box<Command>,
    },

    // ==================== Bulk Operations ====================
    /// Execute a batch of commands
    Batch { commands: Vec<Command> },

    /// Bulk insert documents
    BulkInsert {
        database: String,
        collection: String,
        documents: Vec<Value>,
    },

    // ==================== Script Management ====================
    /// Create a new script
    CreateScript {
        database: String,
        name: String,
        path: String,
        #[serde(default)]
        methods: Vec<String>,
        code: String,
        description: Option<String>,
        collection: Option<String>,
    },

    /// List all scripts
    ListScripts { database: String },

    /// Get a script by ID
    GetScript { database: String, script_id: String },

    /// Update a script
    UpdateScript {
        database: String,
        script_id: String,
        name: Option<String>,
        path: Option<String>,
        methods: Option<Vec<String>>,
        code: Option<String>,
        description: Option<String>,
    },

    /// Delete a script
    DeleteScript { database: String, script_id: String },

    /// Get script runtime statistics
    GetScriptStats,

    // ==================== Job/Queue Management ====================
    /// List all job queues
    ListQueues { database: String },

    /// List jobs in a queue
    ListJobs {
        database: String,
        queue_name: String,
        status: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    },

    /// Enqueue a background job
    EnqueueJob {
        database: String,
        queue_name: String,
        script_path: String,
        params: Option<Value>,
        priority: Option<i32>,
        run_at: Option<i64>,     // Timestamp in ms
        max_retries: Option<u32>,
    },

    /// Cancel a job
    CancelJob { database: String, job_id: String },

    // ==================== Cron Job Management ====================
    /// List all cron jobs
    ListCronJobs { database: String },

    /// Create a new cron job
    CreateCronJob {
        database: String,
        name: String,
        cron_expression: String,
        script_path: String,
        params: Option<Value>,
        queue: Option<String>,
        priority: Option<i32>,
        max_retries: Option<u32>,
    },

    /// Update a cron job
    UpdateCronJob {
        database: String,
        cron_id: String,
        name: Option<String>,
        cron_expression: Option<String>,
        script_path: Option<String>,
        params: Option<Value>,
        queue: Option<String>,
        priority: Option<i32>,
        max_retries: Option<u32>,
    },

    /// Delete a cron job
    DeleteCronJob { database: String, cron_id: String },

    // ==================== Trigger Management ====================
    /// List all triggers
    ListTriggers { database: String },

    /// List triggers for a collection
    ListCollectionTriggers { database: String, collection: String },

    /// Create a new trigger
    CreateTrigger {
        database: String,
        name: String,
        collection: String,
        events: Vec<String>,
        script_path: String,
        filter: Option<String>,
        queue: Option<String>,
        priority: Option<i32>,
        max_retries: Option<u32>,
        #[serde(default = "default_true")]
        enabled: bool,
    },

    /// Get a trigger by ID
    GetTrigger { database: String, trigger_id: String },

    /// Update a trigger
    UpdateTrigger {
        database: String,
        trigger_id: String,
        name: Option<String>,
        events: Option<Vec<String>>,
        script_path: Option<String>,
        filter: Option<String>,
        queue: Option<String>,
        priority: Option<i32>,
        max_retries: Option<u32>,
        enabled: Option<bool>,
    },

    /// Delete a trigger
    DeleteTrigger { database: String, trigger_id: String },

    /// Toggle a trigger (enable/disable)
    ToggleTrigger { database: String, trigger_id: String },

    // ==================== Environment Variables ====================
    /// List environment variables
    ListEnvVars { database: String },

    /// Set an environment variable
    SetEnvVar {
        database: String,
        key: String,
        value: String,
    },

    /// Delete an environment variable
    DeleteEnvVar { database: String, key: String },

    // ==================== Role Management ====================
    /// List all roles
    ListRoles,

    /// Create a new role
    CreateRole {
        name: String,
        permissions: Vec<String>,
    },

    /// Get a role by name
    GetRole { name: String },

    /// Update a role's permissions
    UpdateRole {
        name: String,
        permissions: Vec<String>,
    },

    /// Delete a role
    DeleteRole { name: String },

    // ==================== User Management ====================
    /// List all users
    ListUsers,

    /// Create a new user
    CreateUser {
        username: String,
        password: String,
        #[serde(default)]
        roles: Vec<String>,
    },

    /// Delete a user
    DeleteUser { username: String },

    /// Get a user's roles
    GetUserRoles { username: String },

    /// Assign a role to a user (optionally scoped to a database)
    AssignRole {
        username: String,
        role: String,
        database: Option<String>,
    },

    /// Revoke a role from a user
    RevokeRole { username: String, role: String },

    /// Get permissions for the current user (requires authentication)
    GetCurrentUserPermissions,

    /// Get current user info (requires authentication)
    GetCurrentUser,

    // ==================== API Key Management ====================
    /// List API keys
    ListApiKeys,

    /// Create a new API key
    CreateApiKey {
        name: String,
        #[serde(default)]
        permissions: Vec<String>,
        expires_at: Option<i64>,
    },

    /// Delete an API key
    DeleteApiKey { key_id: String },

    // ==================== Cluster Management ====================
    /// Get cluster status
    ClusterStatus,

    /// Get cluster info
    ClusterInfo,

    /// Remove a node from the cluster
    ClusterRemoveNode { node_id: String },

    /// Trigger cluster rebalancing
    ClusterRebalance,

    /// Trigger cluster cleanup
    ClusterCleanup,

    /// Reshard database
    ClusterReshard { database: String, shards: u32 },

    // ==================== Advanced Collection Operations ====================
    /// Truncate a collection (remove all documents)
    TruncateCollection { database: String, collection: String },

    /// Compact a collection (reclaim space)
    CompactCollection { database: String, collection: String },

    /// Prune a collection (remove deleted documents history)
    PruneCollection { database: String, collection: String },

    /// Recount collection documents
    RecountCollection { database: String, collection: String },

    /// Repair a collection
    RepairCollection { database: String, collection: String },

    /// Get collection sharding info
    GetCollectionSharding { database: String, collection: String },

    /// Export collection data
    ExportCollection { database: String, collection: String },

    /// Import collection data
    ImportCollection {
        database: String,
        collection: String,
        documents: Vec<Value>,
    },

    /// Set collection schema
    SetCollectionSchema {
        database: String,
        collection: String,
        schema: Value,
    },

    /// Get collection schema
    GetCollectionSchema { database: String, collection: String },

    /// Delete collection schema
    DeleteCollectionSchema { database: String, collection: String },

    // ==================== Advanced Index Operations ====================
    /// Rebuild all indexes for a collection
    RebuildIndexes { database: String, collection: String },

    /// Hybrid search (vector + keyword)
    HybridSearch {
        database: String,
        collection: String,
        query: String,
        vector: Vec<f32>,
        limit: Option<u32>,
        filter: Option<String>,
    },

    // ==================== Geo Index Operations ====================
    /// Create a geo index
    CreateGeoIndex {
        database: String,
        collection: String,
        name: String,
        field: String,
    },

    /// List geo indexes
    ListGeoIndexes { database: String, collection: String },

    /// Delete a geo index
    DeleteGeoIndex {
        database: String,
        collection: String,
        name: String,
    },

    /// Find documents near a point
    GeoNear {
        database: String,
        collection: String,
        field: String,
        latitude: f64,
        longitude: f64,
        radius: Option<f64>,
        limit: Option<i32>, // Changed from u32 to i32 based on previous handler fix
    },

    /// Find documents within a polygon
    GeoWithin {
        database: String,
        collection: String,
        field: String,
        polygon: Vec<(f64, f64)>,
    },

    // ==================== Vector Index Operations ====================
    /// Create a vector index
    CreateVectorIndex {
        database: String,
        collection: String,
        name: String,
        field: String,
        dimensions: i32, // Changed from u32 to i32
        metric: Option<String>,
        ef_construction: Option<i32>, // Changed from u32 to i32
        m: Option<i32>, // Changed from u32 to i32
    },

    /// List vector indexes
    ListVectorIndexes { database: String, collection: String },

    /// Delete a vector index
    DeleteVectorIndex {
        database: String,
        collection: String,
        name: String,
    },

    /// Search similar vectors
    VectorSearch {
        database: String,
        collection: String,
        index_name: String,
        vector: Vec<f32>,
        limit: Option<i32>, // Changed from u32 to i32
        ef_search: Option<i32>, // Changed from u32 to i32
        filter: Option<String>,
    },

    /// Quantize a vector index (optimize for size/speed)
    QuantizeVectorIndex {
        database: String,
        collection: String,
        index_name: String,
    },

    /// Dequantize a vector index (restore precision)
    DequantizeVectorIndex {
        database: String,
        collection: String,
        index_name: String,
    },

    // ==================== TTL Index Operations ====================
    /// Create a TTL index
    CreateTtlIndex {
        database: String,
        collection: String,
        name: String,
        field: String,
        expire_after_seconds: i64, // Changed from u32 to i64
    },

    /// List TTL indexes
    ListTtlIndexes { database: String, collection: String },

    /// Delete a TTL index
    DeleteTtlIndex {
        database: String,
        collection: String,
        name: String,
    },

    // ==================== Columnar Storage ====================
    /// Create a columnar collection (table)
    CreateColumnar {
        database: String,
        name: String,
        columns: Vec<Value>,
    },

    /// List columnar collections
    ListColumnar { database: String },

    /// Get columnar collection status
    GetColumnar { database: String, collection: String },

    /// Delete a columnar collection
    DeleteColumnar { database: String, collection: String },

    /// Insert rows into a columnar collection
    InsertColumnar {
        database: String,
        collection: String,
        rows: Vec<Value>,
    },

    /// Aggregate data in a columnar collection
    AggregateColumnar {
        database: String,
        collection: String,
        aggregations: Vec<Value>,
        group_by: Option<Vec<String>>,
        filter: Option<String>,
    },

    /// Query data from a columnar collection
    QueryColumnar {
        database: String,
        collection: String,
        columns: Option<Vec<String>>,
        filter: Option<String>,
        order_by: Option<String>,
        limit: Option<i32>,
    },

    /// Create an index on a columnar collection
    CreateColumnarIndex {
        database: String,
        collection: String,
        column: String,
    },

    /// List indexes on a columnar collection
    ListColumnarIndexes { database: String, collection: String },

    /// Delete an index on a columnar collection
    DeleteColumnarIndex {
        database: String,
        collection: String,
        column: String,
    },
}

fn default_true() -> bool {
    true
}
