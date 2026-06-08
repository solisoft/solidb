# Collections - browse / create / truncate / drop collections in a database.

class CollectionsController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/collections
  def index
    this._ctx()
    @title = "Collections · " + @db
    this._reset_banners()
    this._load()
  end

  # POST /databases/:db/collections
  def create
    this._ctx()
    name = (params["name"] ?? "").trim()
    if name.blank?
      return this._respond({ "ok": false, "status": 422, "error": "collection name is required" }, "")
    end
    return this._create_columnar(name) if params["type"] == "columnar"
    result = SolidbClient.post_api(SolidbEndpoints.collections(@db), this._build_create_payload(name))
    return this._respond(result, "collection " + name + " created")
  end

  # Columnar collections live behind their own API and require column
  # definitions: [{ "name": "host", "type": "string" }, ...]
  def _create_columnar(name)
    columns_text = (params["columns"] ?? "").trim()
    columns = JSON.parse(columns_text) rescue nil
    if columns.nil? || columns.length() == 0
      return this._respond({ "ok": false, "status": 422,
                             "error": "columnar collections need a JSON array of column definitions" }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.columnar(@db), { "name": name, "columns": columns })
    return this._respond(result, "columnar collection " + name + " created")
  end

  # Optional creation settings from the modal. JSON schema / validation are
  # managed from the collection's documents page, not at creation time.
  def _build_create_payload(name)
    payload = { "name": name }
    coll_type = (params["type"] ?? "").trim()
    payload["type"] = coll_type unless coll_type.blank?
    num_shards = (params["num_shards"] ?? "").trim()
    payload["numShards"] = num_shards.to_int() unless num_shards.blank?
    shard_key = (params["shard_key"] ?? "").trim()
    payload["shardKey"] = shard_key unless shard_key.blank?
    replication_factor = (params["replication_factor"] ?? "").trim()
    payload["replicationFactor"] = replication_factor.to_int() unless replication_factor.blank?
    return payload
  end

  # GET /databases/:db/collections/:name/stats - HTMX-loaded detail fragment
  def stats
    this._ctx()
    @collection_name = params["name"] ?? ""
    result = SolidbClient.get_api(SolidbEndpoints.collection_stats(@db, @collection_name))
    @stats = result["data"] ?? {}
    @stats_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    return render("collections/_stats", { "layout": false })
  end

  # DELETE /databases/:db/collections/:name (?ctype=columnar for columnar)
  def delete
    this._ctx()
    name = params["name"] ?? ""
    if params["ctype"] == "columnar"
      result = SolidbClient.delete_api(SolidbEndpoints.columnar_collection(@db, name))
      return this._respond(result, "columnar collection " + name + " dropped")
    end
    result = SolidbClient.delete_api(SolidbEndpoints.collection(@db, name))
    return this._respond(result, "collection " + name + " dropped")
  end

  # GET /databases/:db/collections/:name/indexes - HTMX-loaded panel
  def indexes
    this._ctx_indexes()
    return this._render_indexes({ "ok": true }, "")
  end

  # POST /databases/:db/collections/:name/indexes
  def create_index
    this._ctx_indexes()
    index_name = (params["index_name"] ?? "").trim()
    fields_text = (params["fields"] ?? "").trim()
    if index_name.blank? || fields_text.blank?
      return this._render_indexes({ "ok": false, "status": 422,
                                    "error": "index name and fields are required" }, "")
    end
    index_type = (params["index_type"] ?? "persistent").trim()
    result = this._create_index_request(index_name, fields_text, index_type)
    return this._render_indexes(result, "index " + index_name + " created")
  end

  # PUT /databases/:db/collections/:name/indexes/rebuild
  def rebuild_indexes
    this._ctx_indexes()
    result = SolidbClient.put_api(SolidbEndpoints.collection_indexes_rebuild(@db, @collection_name))
    indexed_count = (result["data"] ?? {})["documents_indexed"] ?? 0
    return this._render_indexes(result, "indexes rebuilt (" + str(indexed_count) + " documents)")
  end

  # DELETE /databases/:db/collections/:name/indexes/:index_name
  # The unified API drops standard / fulltext / geo / ttl indexes by name.
  def delete_index
    this._ctx_indexes()
    index_name = params["index_name"] ?? ""
    result = SolidbClient.delete_api(SolidbEndpoints.collection_index(@db, @collection_name, index_name))
    return this._render_indexes(result, "index " + index_name + " dropped")
  end

  # Geo and TTL indexes are created through their own API families; everything
  # else (persistent / hash / fulltext / bloom / cuckoo) goes to /index.
  def _create_index_request(index_name, fields_text, index_type)
    fields = []
    for part in fields_text.split(",")
      cleaned = part.trim()
      fields.push(cleaned) unless cleaned.blank?
    end
    if index_type == "geo"
      return SolidbClient.post_api(SolidbEndpoints.geo_indexes(@db, @collection_name),
                                   { "name": index_name, "field": fields[0] })
    end
    if index_type == "ttl"
      expire_text = (params["expire_after_seconds"] ?? "").trim()
      if expire_text.blank?
        return { "ok": false, "status": 422, "error": "ttl indexes need expire_after_seconds" }
      end
      return SolidbClient.post_api(SolidbEndpoints.ttl_indexes(@db, @collection_name),
                                   { "name": index_name, "field": fields[0],
                                     "expire_after_seconds": expire_text.to_int() })
    end
    return SolidbClient.post_api(SolidbEndpoints.collection_indexes(@db, @collection_name),
                                 { "name": index_name, "fields": fields,
                                   "type": index_type, "unique": params["unique"] == "true" })
  end

  def _ctx_indexes
    this._ctx()
    @collection_name = params["name"] ?? ""
  end

  def _render_indexes(result, notice)
    @indexes_error = result["ok"] ? "" : (result["error"] ?? "request failed")
    @indexes_notice = result["ok"] ? notice : ""
    this._load_indexes()
    return render("collections/_indexes", { "layout": false })
  end

  # Merge the three index families (standard+fulltext, geo, ttl) into one
  # display list of { name, fields, type, unique, detail } rows.
  def _load_indexes
    @indexes = []
    result = SolidbClient.get_api(SolidbEndpoints.collection_indexes(@db, @collection_name))
    if !result["ok"]
      @indexes_error = result["error"] ?? "request failed" if @indexes_error.blank?
      return
    end
    for entry in ((result["data"] ?? {})["indexes"] ?? [])
      type_name = str(entry["index_type"] ?? "persistent").downcase()
      @indexes.push({ "name": entry["name"] ?? "", "fields": entry["fields"] ?? [],
                      "type": type_name, "unique": entry["unique"] ?? false,
                      "detail": str(entry["indexed_documents"] ?? 0) + " docs" })
    end
    geo_result = SolidbClient.get_api(SolidbEndpoints.geo_indexes(@db, @collection_name))
    for entry in ((geo_result["data"] ?? {})["indexes"] ?? [])
      @indexes.push({ "name": entry["name"] ?? "", "fields": [entry["field"] ?? ""],
                      "type": "geo", "unique": false,
                      "detail": str(entry["indexed_documents"] ?? 0) + " docs" })
    end
    ttl_result = SolidbClient.get_api(SolidbEndpoints.ttl_indexes(@db, @collection_name))
    for entry in ((ttl_result["data"] ?? {})["indexes"] ?? [])
      @indexes.push({ "name": entry["name"] ?? "", "fields": [entry["field"] ?? ""],
                      "type": "ttl", "unique": false,
                      "detail": "expires after " + str(entry["expire_after_seconds"] ?? 0) + "s" })
    end
  end

  # Route context: set explicitly per action (before_action hooks are wired
  # by a startup-time scan and are unreliable under dev hot-reload).
  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
  end

  def _load
    result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    regular = (result["data"] ?? {})["collections"] ?? []
    # Columnar collections appear in the regular list as internal
    # `_columnar_<name>` document collections -- hide those and merge the
    # real entries from the columnar API instead.
    @collections = regular.filter do |coll| !(coll["name"] ?? "").starts_with("_columnar_") end
    columnar_result = SolidbClient.get_api(SolidbEndpoints.columnar(@db))
    columnar_entries = (columnar_result["data"] ?? {})["collections"] ?? []
    for entry in columnar_entries
      @collections.push({ "name": entry["name"] ?? "", "type": "columnar",
                          "count": entry["row_count"] ?? 0, "stats": {} })
    end
    if !result["ok"]
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  def _respond(result, notice)
    @title = "Collections · " + @db
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("collections/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("collections/index")
  end
end
