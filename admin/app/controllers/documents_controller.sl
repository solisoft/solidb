# Documents - browse a collection's documents with an SDBQL filter and
# offset/limit pagination, plus create / edit / delete single documents.

class DocumentsController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/collections/:name/docs
  def index
    this._ctx()
    @title = @collection_name + " · " + @db
    this._reset_banners()
    this._load()
  end

  # POST /databases/:db/collections/:name/docs
  def create
    this._ctx()
    document = this._parse_document()
    if document.nil?
      return this._respond({ "ok": false, "status": 422, "error": "document must be a JSON object" }, "")
    end
    result = SolidbClient.post_api(SolidbEndpoints.documents(@db, @collection_name), document)
    return this._respond(result, "document created")
  end

  # PUT /databases/:db/collections/:name/docs/:key
  def update
    this._ctx()
    document = this._parse_document()
    if document.nil?
      return this._respond({ "ok": false, "status": 422, "error": "document must be a JSON object" }, "")
    end
    key = params["key"] ?? ""
    result = SolidbClient.put_api(SolidbEndpoints.document(@db, @collection_name, key), document)
    return this._respond(result, "document " + key + " updated")
  end

  # PUT /databases/:db/collections/:name/docs/truncate - remove every document
  # but keep the collection (and its indexes) in place.
  def truncate
    this._ctx()
    result = SolidbClient.put_api(SolidbEndpoints.collection_truncate(@db, @collection_name))
    return this._respond(result, "collection " + @collection_name + " truncated")
  end

  # PUT /databases/:db/collections/:name/docs/schema - set / replace the
  # collection's JSON schema and validation mode.
  def update_schema
    this._ctx()
    schema = JSON.parse((params["schema"] ?? "").trim()) rescue nil
    if schema.nil? || type(schema) != "hash"
      return this._respond({ "ok": false, "status": 422, "error": "schema must be a valid JSON object" }, "")
    end
    mode = (params["validation_mode"] ?? "off").trim()
    mode = "off" unless ["off", "lenient", "strict"].includes?(mode)
    result = SolidbClient.post_api(SolidbEndpoints.collection_schema(@db, @collection_name),
                                   { "schema": schema, "validationMode": mode })
    # _ctx captured the pre-update schema; reload so the page reflects the write.
    this._load_schema()
    return this._respond(result, "schema updated (" + mode + ")")
  end

  # DELETE /databases/:db/collections/:name/docs/schema
  def delete_schema
    this._ctx()
    result = SolidbClient.delete_api(SolidbEndpoints.collection_schema(@db, @collection_name))
    this._load_schema()
    return this._respond(result, "schema removed")
  end

  # POST /databases/:db/collections/:name/docs/upload - multipart file upload
  # into a blob collection. The file's base64 payload goes through the binary
  # driver protocol (store_blob), never through an HTTP string body, so the
  # bytes arrive exactly as sent.
  def upload
    this._ctx()
    wants_json = (req["headers"]["accept"] ?? "").includes?("application/json")
    file = find_uploaded_file(req, "file")
    if file.nil?
      return render_json({ "ok": false, "error": "no file selected" }) if wants_json
      return this._respond({ "ok": false, "status": 422, "error": "no file selected" }, "")
    end
    blob_id = nil
    try
      conn = this._driver()
      blob_id = conn.store_blob(@collection_name, file["data"],
                                file["filename"] ?? "upload.bin",
                                file["content_type"] ?? "application/octet-stream")
    catch upload_error
      return render_json({ "ok": false, "error": str(upload_error) }) if wants_json
      return this._respond({ "ok": false, "status": 500, "error": str(upload_error) }, "")
    end
    return render_json({ "ok": true, "blob_id": blob_id }) if wants_json
    return this._respond({ "ok": true }, "file " + (file["filename"] ?? "") + " uploaded (" + str(blob_id) + ")")
  end

  # GET /databases/:db/collections/:name/docs/:key/blob - stream a blob back
  # (inline for preview, attachment with ?download=1).
  def blob
    this._ctx()
    key = params["key"] ?? ""
    meta = nil
    data_base64 = nil
    try
      conn = this._driver()
      meta = conn.get_blob_metadata(@collection_name, key)
      data_base64 = conn.get_blob(@collection_name, key)
    catch blob_error
      meta = nil
    end
    # The driver returns null (rather than raising) for unknown keys.
    if meta == nil || data_base64 == nil
      return { "status": 404, "headers": { "Content-Type": "text/plain" }, "body": "blob not found" }
    end
    # Blob metadata fields vary by upload path: `name`/`type` are canonical,
    # `filename`/`content_type` only present on some.
    #
    # Both of these are attacker-controlled: any principal with write access
    # to a blob collection sets them, and an admin later opens the preview.
    # Echoing the stored content type back with `inline` on this origin turned
    # an uploaded `text/html` (or SVG) blob into stored XSS against the admin
    # UI -- which holds a SoliDB admin JWT and can reach the Lua REPL. So only
    # a small allowlist of image types is ever rendered inline; everything
    # else is downloaded as an opaque octet-stream.
    declared_type = meta["content_type"] ?? (meta["type"] ?? "application/octet-stream")
    filename = this._safe_filename(meta["filename"] ?? (meta["name"] ?? key))
    wants_download = params["download"] == "1"
    # Raster image types only -- `image/svg+xml` is a script execution
    # context, so it is deliberately absent.
    inline_types = ["image/png", "image/jpeg", "image/jpg", "image/gif",
                    "image/webp", "image/avif", "image/bmp"]
    inline_ok = !wants_download && inline_types.includes?(declared_type.downcase().trim())
    content_type = inline_ok ? declared_type.downcase().trim() : "application/octet-stream"
    disposition = inline_ok ? "inline" : "attachment"
    return {
      "status": 200,
      "headers": {
        "Content-Type": content_type,
        "Content-Disposition": disposition + "; filename=\"" + filename + "\"",
        "X-Content-Type-Options": "nosniff",
        # Belt and braces: even for an allowlisted image type, deny the
        # response any ability to execute or load anything.
        "Content-Security-Policy": "default-src 'none'; img-src 'self' data:; sandbox",
        "Cache-Control": "no-store"
      },
      "body": Base64.decode(data_base64)
    }
  end

  # Strip anything that could break out of the quoted Content-Disposition
  # filename or inject a header.
  def _safe_filename(name)
    cleaned = str(name).gsub("[^A-Za-z0-9._ -]", "_").trim()
    return "download" if cleaned.blank?
    return cleaned.substring(0, 200)
  end

  # GET /databases/:db/collections/:name/docs/export - stream the collection
  # as a .jsonl download (proxied from the SoliDB export API). Document
  # collections only: blob exports interleave raw binary chunks that would be
  # corrupted by string transport. (`export` is a reserved keyword in Soli,
  # hence export_docs.)
  def export_docs
    this._ctx()
    if @collection_type == "blob"
      return this._respond({ "ok": false, "status": 422, "error": "blob collections cannot be exported as JSONL" }, "")
    end
    result = SolidbClient.get_raw(SolidbEndpoints.collection_export(@db, @collection_name))
    if !result["ok"]
      return this._respond(result, "")
    end
    return {
      "status": 200,
      "headers": {
        "Content-Type": "application/x-ndjson",
        "Content-Disposition": "attachment; filename=\"" + @db + "-" + @collection_name + ".jsonl\""
      },
      "body": result["body"]
    }
  end

  # POST /databases/:db/collections/:name/docs/import - multipart upload of a
  # .json (array or single object, pretty-printed ok) or .jsonl file. The
  # content is normalized to JSONL before hitting the import API, whose
  # format sniffing chokes on pretty-printed JSON. (`import` is a reserved
  # keyword in Soli, hence import_docs.)
  def import_docs
    this._ctx()
    file = find_uploaded_file(req, "file")
    if file.nil?
      return this._respond({ "ok": false, "status": 422, "error": "no file selected" }, "")
    end
    content = Base64.decode(file["data"]) rescue ""
    jsonl = this._to_jsonl(content)
    if jsonl.blank?
      return this._respond({ "ok": false, "status": 422, "error": "file is empty" }, "")
    end
    result = SolidbClient.post_multipart(SolidbEndpoints.collection_import(@db, @collection_name),
                                         file["filename"] ?? "import.jsonl", jsonl)
    imported = (result["data"] ?? {})["count"] ?? 0
    failed = (result["data"] ?? {})["failed"] ?? 0
    notice = "imported " + str(imported) + " document(s)"
    notice = notice + ", " + str(failed) + " failed" if failed > 0
    return this._respond(result, notice)
  end

  # DELETE /databases/:db/collections/:name/docs/:key
  def delete
    this._ctx()
    key = params["key"] ?? ""
    result = SolidbClient.delete_api(SolidbEndpoints.document(@db, @collection_name, key))
    return this._respond(result, "document " + key + " deleted")
  end

  def _ctx
    @db = params["db"] ?? ""
    @collection_name = params["name"] ?? ""
    @databases = AdminContext.database_names()
    @filter = (params["filter"] ?? "").trim()
    @filter_rejected = false
    # The filter is spliced into query text that runs with the admin JWT this
    # app holds, on a GET route -- and the framework's CSRF gate exempts safe
    # methods. So a plain <img src="...?filter=..."> in any page an admin
    # views executed whatever the filter said. SDBQL accepts a mutation after
    # FILTER and treats `--` as a line comment, which is all
    # `true REMOVE doc IN users --` needs to empty a collection.
    unless this._safe_filter(@filter)
      @filter_rejected = true
      @filter = ""
    end
    @limit = this._page_limit()
    offset = (params["offset"] ?? "0").to_int()
    offset = 0 if offset < 0
    @offset = offset
    this._load_collection_type()
    this._load_schema()
  end

  # blob collections get upload/download/preview affordances in the view.
  def _load_collection_type
    result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    collections = (result["data"] ?? {})["collections"] ?? []
    matching = collections.filter do |coll| coll["name"] == @collection_name end
    @collection_type = matching.length() > 0 ? (matching[0]["type"] ?? "document") : "document"
  end

  # Current JSON schema + validation mode, feeding the schema editor modal.
  # Blob collections store binary chunks and never carry a schema.
  def _load_schema
    @schema_json = ""
    @schema_mode = "off"
    return if @collection_type == "blob"
    result = SolidbClient.get_api(SolidbEndpoints.collection_schema(@db, @collection_name))
    schema = (result["data"] ?? {})["schema"]
    @schema_json = JSON.stringify(schema) unless schema.nil?
    @schema_mode = (result["data"] ?? {})["validationMode"] ?? "off"
  end

  # Authenticated binary-protocol connection (byte-safe for blob payloads).
  def _driver
    conn = Solidb(SolidbClient.host(), @db)
    conn.auth(SolidbClient.username(), SolidbClient.password())
    return conn
  end

  def _page_limit
    limit = (params["limit"] ?? "25").to_int()
    return 25 unless [25, 50, 100].includes?(limit)
    return limit
  end

  # nil when the document textarea holds invalid JSON.
  def _parse_document
    document = JSON.parse((params["document"] ?? "").trim()) rescue nil
    return document
  end

  # Normalize an uploaded file to JSONL. A parseable JSON array becomes one
  # compact line per element; a single object becomes one line. Content that
  # is not whole-file JSON (i.e. multi-line JSONL) passes through - the
  # import API validates line by line. Every return path ends with "\n":
  # the streaming importer silently drops a final unterminated line.
  def _to_jsonl(content)
    text = (content ?? "").trim()
    return "" if text.blank?
    parsed = JSON.parse(text) rescue nil
    return text + "\n" if parsed.nil?
    if type(parsed) == "array"
      lines = parsed.map do |doc| JSON.stringify(doc) end
      return lines.join("\n") + "\n"
    end
    return JSON.stringify(parsed) + "\n"
  end

  # True when `text` is safe to splice after FILTER.
  #
  # Clause keywords and comment markers must never appear in a browse filter.
  # A filter is an *expression*; every keyword below starts a new clause,
  # opens a comment, or names a catalog-mutating builtin, and each is a way
  # out of the FILTER and into a write.
  #
  # `IN` is deliberately absent: `doc.status IN ["a", "b"]` is an ordinary
  # filter, and a mutation needs one of the words below to get going.
  def _safe_filter(text)
    return true if text.blank?
    denylist = ["for", "let", "collect", "insert", "update", "upsert",
                "replace", "remove", "return", "into", "create", "drop",
                "refresh", "materialized", "view", "graph", "stream",
                "with", "union", "intersect", "except", "window",
                "create_view", "drop_view", "create_graph", "drop_graph"]
    lowered = text.downcase()
    # Comments would let a payload discard the rest of the generated query.
    return false if lowered.includes?("--")
    return false if lowered.includes?("/*")
    return false if lowered.includes?("*/")
    # Split on anything that is not part of an identifier, so keywords are
    # matched as whole words rather than as substrings of a field name
    # (`doc.created_for` must stay usable).
    words = lowered.gsub("[^a-z0-9_]", " ").split(" ")
    # `for`, not `.each`: a `return` inside a block leaves the block, not the
    # method, so an `.each` here would fall through to `return true` and the
    # guard would never reject anything.
    for word in words
      return false if denylist.includes?(word)
    end
    return true
  end

  # Runs the listing query: the filter input is spliced as a FILTER clause on
  # `doc`, offset/limit are bound. Fetches limit+1 rows to know has-more.
  # Blob collections project metadata only -- their docs embed binary chunk
  # data that must never be rendered into HTML.
  def _load
    return_clause = " LIMIT @offset, @batch RETURN doc"
    if @collection_type == "blob"
      return_clause = " LIMIT @offset, @batch RETURN { \"_key\": doc._key, \"filename\": doc.filename," +
                      " \"name\": doc.name, \"size\": doc.size, \"content_type\": doc.content_type," +
                      " \"type\": doc.type, \"chunks\": doc.chunks, \"created\": doc.created }"
    end
    query = "FOR doc IN " + @collection_name
    query = query + " FILTER " + @filter unless @filter.blank?
    query = query + return_clause
    # cache: false - the browser must reflect writes that bypass the query
    # cache (bulk import's insert_batch does not invalidate it).
    payload = { "query": query, "bindVars": { "offset": @offset, "batch": @limit + 1 }, "cache": false }
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db), payload)
    rows = (result["data"] ?? {})["result"] ?? []
    @has_more = rows.length() > @limit
    @documents = @has_more ? rows.slice(0, @limit) : rows
    @query_error = ""
    if !result["ok"]
      @query_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
  end

  def _reset_banners
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  def _respond(result, notice)
    @title = @collection_name + " · " + @db
    this._reset_banners()
    if result["ok"]
      @flash_notice = notice
    else
      @flash_error = result["error"] ?? "request failed"
      @solidb_down = (result["status"] ?? -1) == 0
    end
    this._load()
    return render("documents/index", { "layout": false }) if req["headers"]["hx-request"] == "true"
    return render("documents/index")
  end
end
