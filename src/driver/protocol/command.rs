use super::types::IsolationLevel;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    Auth {
        database: String,
        username: String,
        password: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
    },
    Ping,
    ListDatabases,
    CreateDatabase {
        name: String,
    },
    DeleteDatabase {
        name: String,
    },
    ListCollections {
        database: String,
    },
    CreateCollection {
        database: String,
        name: String,
        #[serde(rename = "type")]
        collection_type: Option<String>,
    },
    DeleteCollection {
        database: String,
        name: String,
    },
    CollectionStats {
        database: String,
        name: String,
    },
    Get {
        database: String,
        collection: String,
        key: String,
    },
    Insert {
        database: String,
        collection: String,
        key: Option<String>,
        document: Value,
    },
    Update {
        database: String,
        collection: String,
        key: String,
        document: Value,
        #[serde(default)]
        merge: bool,
    },
    Delete {
        database: String,
        collection: String,
        key: String,
    },
    List {
        database: String,
        collection: String,
        limit: Option<usize>,
        offset: Option<usize>,
    },
    Query {
        database: String,
        sdbql: String,
        bind_vars: Option<std::collections::HashMap<String, Value>>,
    },
    Explain {
        database: String,
        sdbql: String,
        bind_vars: Option<std::collections::HashMap<String, Value>>,
    },
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
    DeleteIndex {
        database: String,
        collection: String,
        name: String,
    },
    ListIndexes {
        database: String,
        collection: String,
    },
    BeginTransaction {
        database: String,
        #[serde(default)]
        isolation_level: IsolationLevel,
    },
    CommitTransaction {
        tx_id: String,
    },
    RollbackTransaction {
        tx_id: String,
    },
    TransactionCommand {
        tx_id: String,
        command: Box<Command>,
    },
    Batch {
        commands: Vec<Command>,
    },
    BulkInsert {
        database: String,
        collection: String,
        documents: Vec<Value>,
    },
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
    ListScripts {
        database: String,
    },
    GetScript {
        database: String,
        script_id: String,
    },
    UpdateScript {
        database: String,
        script_id: String,
        name: Option<String>,
        path: Option<String>,
        methods: Option<Vec<String>>,
        code: Option<String>,
        description: Option<String>,
    },
    DeleteScript {
        database: String,
        script_id: String,
    },
    GetScriptStats,
    ListQueues {
        database: String,
    },
    ListJobs {
        database: String,
        queue_name: String,
        status: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    },
    EnqueueJob {
        database: String,
        queue_name: String,
        script_path: String,
        params: Option<Value>,
        priority: Option<i32>,
        run_at: Option<i64>,
        max_retries: Option<u32>,
    },
    CancelJob {
        database: String,
        job_id: String,
    },
    ListCronJobs {
        database: String,
    },
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
    DeleteCronJob {
        database: String,
        cron_id: String,
    },
    ListTriggers {
        database: String,
    },
    ListCollectionTriggers {
        database: String,
        collection: String,
    },
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
    GetTrigger {
        database: String,
        trigger_id: String,
    },
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
    DeleteTrigger {
        database: String,
        trigger_id: String,
    },
    ToggleTrigger {
        database: String,
        trigger_id: String,
    },
    ListEnvVars {
        database: String,
    },
    SetEnvVar {
        database: String,
        key: String,
        value: String,
    },
    DeleteEnvVar {
        database: String,
        key: String,
    },
    ListRoles,
    CreateRole {
        name: String,
        permissions: Vec<String>,
    },
    GetRole {
        name: String,
    },
    UpdateRole {
        name: String,
        permissions: Vec<String>,
    },
    DeleteRole {
        name: String,
    },
    ListUsers,
    CreateUser {
        username: String,
        password: String,
        #[serde(default)]
        roles: Vec<String>,
    },
    DeleteUser {
        username: String,
    },
    GetUserRoles {
        username: String,
    },
    AssignRole {
        username: String,
        role: String,
        database: Option<String>,
    },
    RevokeRole {
        username: String,
        role: String,
    },
    GetCurrentUserPermissions,
    GetCurrentUser,
    ListApiKeys,
    CreateApiKey {
        name: String,
        #[serde(default)]
        permissions: Vec<String>,
        expires_at: Option<i64>,
    },
    DeleteApiKey {
        key_id: String,
    },
    ClusterStatus,
    ClusterInfo,
    ClusterRemoveNode {
        node_id: String,
    },
    ClusterRebalance,
    ClusterCleanup,
    ClusterReshard {
        database: String,
        shards: u32,
    },
    TruncateCollection {
        database: String,
        collection: String,
    },
    CompactCollection {
        database: String,
        collection: String,
    },
    PruneCollection {
        database: String,
        collection: String,
    },
    RecountCollection {
        database: String,
        collection: String,
    },
    RepairCollection {
        database: String,
        collection: String,
    },
    GetCollectionSharding {
        database: String,
        collection: String,
    },
    ExportCollection {
        database: String,
        collection: String,
    },
    ImportCollection {
        database: String,
        collection: String,
        documents: Vec<Value>,
    },
    SetCollectionSchema {
        database: String,
        collection: String,
        schema: Value,
    },
    GetCollectionSchema {
        database: String,
        collection: String,
    },
    DeleteCollectionSchema {
        database: String,
        collection: String,
    },
    RebuildIndexes {
        database: String,
        collection: String,
    },
    HybridSearch {
        database: String,
        collection: String,
        vector: Vec<f32>,
        text_query: String,
        vector_index: String,
        fulltext_field: String,
        #[serde(default)]
        vector_weight: Option<f32>,
        #[serde(default)]
        text_weight: Option<f32>,
        #[serde(default)]
        limit: Option<u32>,
        #[serde(default)]
        fusion: Option<String>,
    },
    GraphNeighbors {
        database: String,
        edge_collection: String,
        seeds: serde_json::Value,
        #[serde(default)]
        options: Option<serde_json::Value>,
    },
    GraphRag {
        database: String,
        seed_collection: String,
        vector_index: String,
        edge_collection: String,
        query_vector: Vec<f32>,
        #[serde(default)]
        options: Option<serde_json::Value>,
    },
    CommunitySearch {
        database: String,
        query_text: String,
        #[serde(default)]
        options: Option<serde_json::Value>,
    },
    CreateGeoIndex {
        database: String,
        collection: String,
        name: String,
        field: String,
    },
    ListGeoIndexes {
        database: String,
        collection: String,
    },
    DeleteGeoIndex {
        database: String,
        collection: String,
        name: String,
    },
    GeoNear {
        database: String,
        collection: String,
        field: String,
        latitude: f64,
        longitude: f64,
        radius: Option<f64>,
        limit: Option<i32>,
    },
    GeoWithin {
        database: String,
        collection: String,
        field: String,
        polygon: Vec<(f64, f64)>,
    },
    CreateVectorIndex {
        database: String,
        collection: String,
        name: String,
        field: String,
        dimensions: i32,
        metric: Option<String>,
        ef_construction: Option<i32>,
        m: Option<i32>,
    },
    ListVectorIndexes {
        database: String,
        collection: String,
    },
    DeleteVectorIndex {
        database: String,
        collection: String,
        name: String,
    },
    VectorSearch {
        database: String,
        collection: String,
        index_name: String,
        vector: Vec<f32>,
        limit: Option<i32>,
        ef_search: Option<i32>,
        filter: Option<String>,
    },
    QuantizeVectorIndex {
        database: String,
        collection: String,
        index_name: String,
    },
    DequantizeVectorIndex {
        database: String,
        collection: String,
        index_name: String,
    },
    CreateTtlIndex {
        database: String,
        collection: String,
        name: String,
        field: String,
        expire_after_seconds: i64,
    },
    ListTtlIndexes {
        database: String,
        collection: String,
    },
    DeleteTtlIndex {
        database: String,
        collection: String,
        name: String,
    },
    CreateColumnar {
        database: String,
        name: String,
        columns: Vec<Value>,
    },
    ListColumnar {
        database: String,
    },
    GetColumnar {
        database: String,
        collection: String,
    },
    DeleteColumnar {
        database: String,
        collection: String,
    },
    InsertColumnar {
        database: String,
        collection: String,
        rows: Vec<Value>,
    },
    AggregateColumnar {
        database: String,
        collection: String,
        aggregations: Vec<Value>,
        group_by: Option<Vec<String>>,
        filter: Option<String>,
    },
    QueryColumnar {
        database: String,
        collection: String,
        columns: Option<Vec<String>>,
        filter: Option<String>,
        order_by: Option<String>,
        limit: Option<i32>,
    },
    CreateColumnarIndex {
        database: String,
        collection: String,
        column: String,
    },
    ListColumnarIndexes {
        database: String,
        collection: String,
    },
    DeleteColumnarIndex {
        database: String,
        collection: String,
        column: String,
    },
}

fn default_true() -> bool {
    true
}

impl Command {
    /// Database this command targets, when it carries one. Global commands
    /// (database/role/user/key/cluster management) return `None`.
    pub fn database(&self) -> Option<&str> {
        use Command::*;
        match self {
            ListCollections { database }
            | CreateCollection { database, .. }
            | DeleteCollection { database, .. }
            | CollectionStats { database, .. }
            | Get { database, .. }
            | Insert { database, .. }
            | Update { database, .. }
            | Delete { database, .. }
            | List { database, .. }
            | Query { database, .. }
            | Explain { database, .. }
            | CreateIndex { database, .. }
            | DeleteIndex { database, .. }
            | ListIndexes { database, .. }
            | BeginTransaction { database, .. }
            | BulkInsert { database, .. }
            | CreateScript { database, .. }
            | ListScripts { database }
            | GetScript { database, .. }
            | UpdateScript { database, .. }
            | DeleteScript { database, .. }
            | ListQueues { database }
            | ListJobs { database, .. }
            | EnqueueJob { database, .. }
            | CancelJob { database, .. }
            | ListCronJobs { database }
            | CreateCronJob { database, .. }
            | UpdateCronJob { database, .. }
            | DeleteCronJob { database, .. }
            | ListTriggers { database }
            | ListCollectionTriggers { database, .. }
            | CreateTrigger { database, .. }
            | GetTrigger { database, .. }
            | UpdateTrigger { database, .. }
            | DeleteTrigger { database, .. }
            | ToggleTrigger { database, .. }
            | ListEnvVars { database }
            | SetEnvVar { database, .. }
            | DeleteEnvVar { database, .. }
            | ClusterReshard { database, .. }
            | TruncateCollection { database, .. }
            | CompactCollection { database, .. }
            | PruneCollection { database, .. }
            | RecountCollection { database, .. }
            | RepairCollection { database, .. }
            | GetCollectionSharding { database, .. }
            | ExportCollection { database, .. }
            | ImportCollection { database, .. }
            | SetCollectionSchema { database, .. }
            | GetCollectionSchema { database, .. }
            | DeleteCollectionSchema { database, .. }
            | RebuildIndexes { database, .. }
            | HybridSearch { database, .. }
            | GraphNeighbors { database, .. }
            | GraphRag { database, .. }
            | CommunitySearch { database, .. }
            | CreateGeoIndex { database, .. }
            | ListGeoIndexes { database, .. }
            | DeleteGeoIndex { database, .. }
            | GeoNear { database, .. }
            | GeoWithin { database, .. }
            | CreateVectorIndex { database, .. }
            | ListVectorIndexes { database, .. }
            | DeleteVectorIndex { database, .. }
            | VectorSearch { database, .. }
            | QuantizeVectorIndex { database, .. }
            | DequantizeVectorIndex { database, .. }
            | CreateTtlIndex { database, .. }
            | ListTtlIndexes { database, .. }
            | DeleteTtlIndex { database, .. }
            | CreateColumnar { database, .. }
            | ListColumnar { database }
            | GetColumnar { database, .. }
            | DeleteColumnar { database, .. }
            | InsertColumnar { database, .. }
            | AggregateColumnar { database, .. }
            | QueryColumnar { database, .. }
            | CreateColumnarIndex { database, .. }
            | ListColumnarIndexes { database, .. }
            | DeleteColumnarIndex { database, .. } => Some(database),
            TransactionCommand { command, .. } => command.database(),
            _ => None,
        }
    }

    /// Permission this command requires, mirroring the HTTP authz
    /// middleware's mapping (reads → Read; mutations → Write; truncate /
    /// drop-collection and global management commands → Admin).
    /// Returns `None` for commands exempt from the per-command check:
    /// Auth/Ping, Batch (inner commands are re-dispatched individually),
    /// commit/rollback (their mutations were checked when issued), and
    /// self-introspection.
    pub fn required_action(&self) -> Option<crate::server::authorization::PermissionAction> {
        use crate::server::authorization::PermissionAction as A;
        use Command::*;
        match self {
            Auth { .. }
            | Ping
            | Batch { .. }
            | CommitTransaction { .. }
            | RollbackTransaction { .. }
            | GetCurrentUser
            | GetCurrentUserPermissions => None,

            TransactionCommand { command, .. } => command.required_action(),

            // Reads. Mutating SDBQL via `Query` is upgraded to Write in
            // `handle_query`, same as the HTTP /cursor endpoint.
            ListDatabases
            | ListCollections { .. }
            | CollectionStats { .. }
            | Get { .. }
            | List { .. }
            | Query { .. }
            | Explain { .. }
            | ListIndexes { .. }
            | ListScripts { .. }
            | GetScript { .. }
            | GetScriptStats
            | ListQueues { .. }
            | ListJobs { .. }
            | ListCronJobs { .. }
            | ListTriggers { .. }
            | ListCollectionTriggers { .. }
            | GetTrigger { .. }
            | ListEnvVars { .. }
            | GetCollectionSharding { .. }
            | ExportCollection { .. }
            | GetCollectionSchema { .. }
            | HybridSearch { .. }
            | CommunitySearch { .. }
            | ListGeoIndexes { .. }
            | GeoNear { .. }
            | GeoWithin { .. }
            | ListVectorIndexes { .. }
            | VectorSearch { .. }
            | ListTtlIndexes { .. }
            | ListColumnar { .. }
            | GetColumnar { .. }
            | AggregateColumnar { .. }
            | QueryColumnar { .. }
            | ListColumnarIndexes { .. } => Some(A::Read),

            // Graph expansion lazily creates the `_from`/`_to` indexes an edge
            // collection needs, so it is a Write — same as its HTTP
            // counterparts, which fall through to Write for lack of a
            // read-semantics suffix. `CommunitySearch` only reads a prior
            // build's output, so it stays a Read (as does `POST
            // /graph/community/search`, matched by the `/search` suffix).
            GraphNeighbors { .. } | GraphRag { .. } => Some(A::Write),

            // Irreversible bulk destruction — Admin, same as HTTP
            TruncateCollection { .. } | DeleteCollection { .. } | DeleteColumnar { .. } => {
                Some(A::Admin)
            }

            // Global management commands — Admin (and denied outright for
            // database-scoped API keys since database() is None)
            CreateDatabase { .. }
            | DeleteDatabase { .. }
            | ListRoles
            | CreateRole { .. }
            | GetRole { .. }
            | UpdateRole { .. }
            | DeleteRole { .. }
            | ListUsers
            | CreateUser { .. }
            | DeleteUser { .. }
            | GetUserRoles { .. }
            | AssignRole { .. }
            | RevokeRole { .. }
            | ListApiKeys
            | CreateApiKey { .. }
            | DeleteApiKey { .. }
            | ClusterStatus
            | ClusterInfo
            | ClusterRemoveNode { .. }
            | ClusterRebalance
            | ClusterCleanup
            | ClusterReshard { .. } => Some(A::Admin),

            // Everything else writes data
            _ => Some(A::Write),
        }
    }
}
