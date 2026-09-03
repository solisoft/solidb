//! Named graphs (`_graphs`) and search views (`_views` type `search`).

use serde_json::{json, Value};

use super::QueryExecutor;
use crate::error::{DbError, DbResult};

const GRAPHS: &str = "_graphs";
const VIEWS: &str = "_views";

impl<'a> QueryExecutor<'a> {
    /// Refuse a catalog write unless the principal may write.
    ///
    /// `CREATE_GRAPH` / `DROP_GRAPH` / `CREATE_VIEW` / `DROP_VIEW` are
    /// ordinary function calls that edit `_graphs` and `_views`, so the
    /// clause-level mutation check never saw them and the routes that reach
    /// them (`/cursor`, `/sql`, `/explain`, the driver's Query op) are
    /// classified Read. A viewer could replace a materialized view's
    /// definition or drop a named graph. `Query::has_mutations` now reports
    /// these calls too, so the middleware upgrades the request; this is the
    /// belt-and-braces check at the point of the write, for executors reached
    /// by any other path.
    ///
    /// Absence of a principal is not permission: an executor built without one
    /// (internal refreshes, jobs, stream tasks) does not edit the catalog
    /// either. Same rule as auto-index creation in `index_opt`.
    fn require_catalog_write(&self, function: &str) -> DbResult<()> {
        match &self.principal {
            Some(p) if p.can_write || p.can_admin => Ok(()),
            _ => Err(DbError::Forbidden(format!(
                "{function} modifies the query catalog and requires write permission"
            ))),
        }
    }
}

impl<'a> QueryExecutor<'a> {
    fn ensure_meta_collection(&self, name: &str) -> DbResult<crate::storage::Collection> {
        match self.get_collection(name) {
            Ok(c) => Ok(c),
            Err(_) => {
                if let Some(ref db) = self.database {
                    let database = self.storage.get_database(db)?;
                    let _ = database.create_collection(name.to_string(), None);
                    database.get_collection(name)
                } else {
                    let _ = self.storage.create_collection(name.to_string(), None);
                    self.storage.get_collection(name)
                }
            }
        }
    }

    /// Resolve `GRAPH name` to an edge collection: catalog first, else the name itself.
    pub(super) fn resolve_edge_collection_name(&self, name: &str) -> DbResult<String> {
        if let Ok(graphs) = self.get_collection(GRAPHS) {
            if let Ok(doc) = graphs.get(name) {
                let v = doc.to_value();
                if let Some(first) = v
                    .get("edges")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                {
                    return Ok(first.to_string());
                }
            }
        }
        Ok(name.to_string())
    }

    /// Search-view backing collection, if `name` is a `type: "search"` `_views` doc.
    pub(super) fn resolve_search_view_collection(&self, name: &str) -> DbResult<Option<String>> {
        if self.get_collection(name).is_ok() {
            return Ok(None);
        }
        let Ok(views) = self.get_collection(VIEWS) else {
            return Ok(None);
        };
        let Ok(doc) = views.get(name) else {
            return Ok(None);
        };
        let v = doc.to_value();
        if v.get("type").and_then(Value::as_str) != Some("search") {
            return Ok(None);
        }
        Ok(v.get("collection")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    pub(super) fn eval_create_graph(&self, args: &[Value]) -> DbResult<Value> {
        self.require_catalog_write("CREATE_GRAPH")?;
        if args.len() < 2 {
            return Err(DbError::ExecutionError(
                "CREATE_GRAPH(name, {vertices, edges}) required".into(),
            ));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| DbError::ExecutionError("CREATE_GRAPH: name must be a string".into()))?;
        let spec = args[1].as_object().ok_or_else(|| {
            DbError::ExecutionError("CREATE_GRAPH: spec must be an object".into())
        })?;
        let vertices = spec.get("vertices").cloned().unwrap_or(json!([]));
        let edges = spec.get("edges").cloned().unwrap_or(json!([]));
        if !vertices.is_array() || !edges.is_array() {
            return Err(DbError::ExecutionError(
                "CREATE_GRAPH: vertices and edges must be arrays".into(),
            ));
        }
        let coll = self.ensure_meta_collection(GRAPHS)?;
        let doc = json!({
            "_key": name,
            "type": "graph",
            "vertices": vertices,
            "edges": edges,
        });
        if coll.get(name).is_ok() {
            coll.delete(name)?;
        }
        coll.insert(doc)?;
        Ok(json!({"name": name, "vertices": vertices, "edges": edges}))
    }

    pub(super) fn eval_drop_graph(&self, args: &[Value]) -> DbResult<Value> {
        self.require_catalog_write("DROP_GRAPH")?;
        let name = args
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| DbError::ExecutionError("DROP_GRAPH(name) required".into()))?;
        let coll = self.ensure_meta_collection(GRAPHS)?;
        coll.delete(name)?;
        Ok(json!(true))
    }

    pub(super) fn eval_graph_info(&self, args: &[Value]) -> DbResult<Value> {
        let name = args
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| DbError::ExecutionError("GRAPH_INFO(name) required".into()))?;
        let coll = self
            .get_collection(GRAPHS)
            .map_err(|_| DbError::ExecutionError(format!("unknown graph '{name}'")))?;
        let doc = coll
            .get(name)
            .map_err(|_| DbError::ExecutionError(format!("unknown graph '{name}'")))?;
        Ok(doc.to_value())
    }

    pub(super) fn eval_create_view(&self, args: &[Value]) -> DbResult<Value> {
        self.require_catalog_write("CREATE_VIEW")?;
        if args.len() < 2 {
            return Err(DbError::ExecutionError(
                "CREATE_VIEW(name, {collection, fields, analyzer?}) required".into(),
            ));
        }
        let name = args[0]
            .as_str()
            .ok_or_else(|| DbError::ExecutionError("CREATE_VIEW: name must be a string".into()))?;
        let spec = args[1]
            .as_object()
            .ok_or_else(|| DbError::ExecutionError("CREATE_VIEW: spec must be an object".into()))?;
        let collection = spec
            .get("collection")
            .and_then(Value::as_str)
            .ok_or_else(|| DbError::ExecutionError("CREATE_VIEW: collection is required".into()))?;
        let fields = spec.get("fields").cloned().unwrap_or(json!([]));
        let analyzer = spec
            .get("analyzer")
            .and_then(Value::as_str)
            .unwrap_or("identity");
        let coll = self.ensure_meta_collection(VIEWS)?;
        let doc = json!({
            "_key": name,
            "type": "search",
            "collection": collection,
            "fields": fields,
            "analyzer": analyzer,
        });
        if coll.get(name).is_ok() {
            coll.delete(name)?;
        }
        coll.insert(doc)?;
        Ok(json!({
            "name": name,
            "type": "search",
            "collection": collection,
            "fields": fields,
            "analyzer": analyzer,
        }))
    }

    pub(super) fn eval_drop_view(&self, args: &[Value]) -> DbResult<Value> {
        self.require_catalog_write("DROP_VIEW")?;
        let name = args
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| DbError::ExecutionError("DROP_VIEW(name) required".into()))?;
        let coll = self.ensure_meta_collection(VIEWS)?;
        let doc = coll.get(name)?;
        if doc.to_value().get("type").and_then(Value::as_str) != Some("search") {
            return Err(DbError::ExecutionError(
                "DROP_VIEW: not a search view".into(),
            ));
        }
        coll.delete(name)?;
        Ok(json!(true))
    }

    pub(super) fn eval_search_index(&self, args: &[Value]) -> DbResult<Value> {
        if args.len() < 3 {
            return Err(DbError::ExecutionError(
                "SEARCH_INDEX(collection, field, query [, limit]) required".into(),
            ));
        }
        let cname = args[0].as_str().ok_or_else(|| {
            DbError::ExecutionError("SEARCH_INDEX: collection must be a string".into())
        })?;
        let field = args[1].as_str().ok_or_else(|| {
            DbError::ExecutionError("SEARCH_INDEX: field must be a string".into())
        })?;
        let query = args[2].as_str().ok_or_else(|| {
            DbError::ExecutionError("SEARCH_INDEX: query must be a string".into())
        })?;
        let limit = args.get(3).and_then(Value::as_u64).unwrap_or(20).min(1000) as usize;
        let collection = self.get_collection(cname)?;
        let hits = collection.fulltext_search(query, Some(vec![field.to_string()]), limit)?;
        let mut out = Vec::with_capacity(hits.len());
        for h in hits {
            let doc = collection
                .get(&h.doc_key)
                .map(|d| d.to_value())
                .unwrap_or(Value::Null);
            out.push(json!({
                "doc": doc,
                "score": h.score,
                "terms": h.matched_terms,
            }));
        }
        Ok(Value::Array(out))
    }

    pub(super) fn eval_can(&self, args: &[Value]) -> DbResult<Value> {
        let action = args
            .first()
            .and_then(Value::as_str)
            .unwrap_or("read")
            .to_ascii_lowercase();
        let Some(p) = self.principal.as_ref() else {
            return Ok(Value::Bool(false));
        };
        let role_ok = match action.as_str() {
            "admin" => p.can_admin,
            "write" => p.can_write || p.can_admin,
            _ => p.can_read || p.can_write || p.can_admin,
        };
        if !role_ok {
            return Ok(Value::Bool(false));
        }
        if args.len() < 2 || args[1].is_null() {
            return Ok(Value::Bool(true));
        }
        let doc = &args[1];
        if let Some(owner) = doc
            .get("owner")
            .or_else(|| doc.get("_owner"))
            .and_then(Value::as_str)
        {
            if owner == p.user {
                return Ok(Value::Bool(true));
            }
        }
        if let Some(acl) = doc.get("_acl") {
            let allowed = acl
                .get(&action)
                .or_else(|| acl.get("*"))
                .and_then(Value::as_array);
            if let Some(arr) = allowed {
                let ok = arr.iter().any(|v| {
                    v.as_str()
                        .map(|s| s == "*" || s == p.user || p.roles.iter().any(|r| r == s))
                        .unwrap_or(false)
                });
                return Ok(Value::Bool(ok));
            }
        }
        // Role grant is enough when the document has no ACL.
        Ok(Value::Bool(true))
    }
}
