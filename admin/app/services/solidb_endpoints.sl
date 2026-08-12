# app/services/solidb_endpoints.sl
#
# Pure SoliDB API path builders. Controllers compose SolidbClient calls with
# these instead of hand-concatenating URL strings, so the paths live in one
# place and every builder is unit-testable.

class SolidbEndpoints
  # --- server / cluster ---
  static def health()
    return "/_api/health"
  end

  static def cluster_info()
    return "/_api/cluster/info"
  end

  static def cluster_status()
    return "/_api/cluster/status"
  end

  static def sync_log_stats()
    return "/_api/cluster/sync-log/stats"
  end

  static def sync_log_prune()
    return "/_api/cluster/sync-log/prune"
  end

  static def livequery_token()
    return "/_api/livequery/token"
  end

  # --- databases ---
  static def databases()
    return "/_api/databases"
  end

  static def database_create()
    return "/_api/database"
  end

  static def database(name)
    return "/_api/database/" + name
  end

  # --- users / roles / api keys ---
  static def users()
    return "/_api/auth/users"
  end

  static def user(username)
    return "/_api/auth/users/" + username
  end

  static def user_roles(username)
    return "/_api/auth/users/" + username + "/roles"
  end

  static def user_role(username, role)
    return "/_api/auth/users/" + username + "/roles/" + role
  end

  static def roles()
    return "/_api/auth/roles"
  end

  static def role(name)
    return "/_api/auth/roles/" + name
  end

  static def api_keys()
    return "/_api/auth/api-keys"
  end

  static def api_key(key_id)
    return "/_api/auth/api-keys/" + key_id
  end

  # --- collections ---
  static def collections(db)
    return "/_api/database/" + db + "/collection"
  end

  static def collection(db, name)
    return "/_api/database/" + db + "/collection/" + name
  end

  static def collection_stats(db, name)
    return SolidbEndpoints.collection(db, name) + "/stats"
  end

  static def collection_properties(db, name)
    return SolidbEndpoints.collection(db, name) + "/properties"
  end

  static def collection_truncate(db, name)
    return SolidbEndpoints.collection(db, name) + "/truncate"
  end

  static def collection_schema(db, name)
    return SolidbEndpoints.collection(db, name) + "/schema"
  end

  static def collection_export(db, name)
    return SolidbEndpoints.collection(db, name) + "/export"
  end

  static def collection_import(db, name)
    return SolidbEndpoints.collection(db, name) + "/import"
  end

  static def collection_prune(db, name)
    return SolidbEndpoints.collection(db, name) + "/prune"
  end

  # --- indexes (standard + fulltext share one family; geo / ttl have
  # their own create+list APIs, but DELETE /index/{coll}/{name} drops any) ---
  static def collection_indexes(db, name)
    return "/_api/database/" + db + "/index/" + name
  end

  static def collection_index(db, name, index_name)
    return SolidbEndpoints.collection_indexes(db, name) + "/" + index_name
  end

  static def collection_indexes_rebuild(db, name)
    return SolidbEndpoints.collection_indexes(db, name) + "/rebuild"
  end

  static def geo_indexes(db, name)
    return "/_api/database/" + db + "/geo/" + name
  end

  static def ttl_indexes(db, name)
    return "/_api/database/" + db + "/ttl/" + name
  end

  static def ttl_index(db, name, index_name)
    return SolidbEndpoints.ttl_indexes(db, name) + "/" + index_name
  end

  # --- columnar collections (separate API family) ---
  static def columnar(db)
    return "/_api/database/" + db + "/columnar"
  end

  static def columnar_collection(db, name)
    return "/_api/database/" + db + "/columnar/" + name
  end

  # --- documents ---
  static def documents(db, collection_name)
    return "/_api/database/" + db + "/document/" + collection_name
  end

  static def document(db, collection_name, key)
    return "/_api/database/" + db + "/document/" + collection_name + "/" + key
  end

  # --- queries ---
  static def cursor(db)
    return "/_api/database/" + db + "/cursor"
  end

  static def explain(db)
    return "/_api/database/" + db + "/explain"
  end

  # --- lua scripts ---
  static def scripts(db)
    return "/_api/database/" + db + "/scripts"
  end

  static def script(db, script_id)
    return "/_api/database/" + db + "/scripts/" + script_id
  end

  # --- triggers ---
  static def triggers(db)
    return "/_api/database/" + db + "/triggers"
  end

  static def trigger(db, trigger_id)
    return "/_api/database/" + db + "/triggers/" + trigger_id
  end

  static def trigger_toggle(db, trigger_id)
    return SolidbEndpoints.trigger(db, trigger_id) + "/toggle"
  end

  # --- lua repl ---
  static def repl(db)
    return "/_api/database/" + db + "/repl"
  end

  # --- env vars ---
  static def env_vars(db)
    return "/_api/database/" + db + "/env"
  end

  static def env_var(db, key)
    return "/_api/database/" + db + "/env/" + key
  end
end
