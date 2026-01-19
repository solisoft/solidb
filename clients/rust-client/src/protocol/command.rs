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
        query: String,
        vector: Vec<f32>,
        limit: Option<u32>,
        filter: Option<String>,
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
