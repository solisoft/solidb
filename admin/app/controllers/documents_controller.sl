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
    content_type = meta["content_type"] ?? (meta["type"] ?? "application/octet-stream")
    filename = meta["filename"] ?? (meta["name"] ?? key)
    disposition = params["download"] == "1" ? "attachment" : "inline"
    return {
      "status": 200,
      "headers": {
        "Content-Type": content_type,
        "Content-Disposition": disposition + "; filename=\"" + filename + "\""
      },
      "body": Base64.decode(data_base64)
    }
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
    @limit = this._page_limit()
    offset = (params["offset"] ?? "0").to_int()
    offset = 0 if offset < 0
    @offset = offset
    this._load_collection_type()
  end

  # blob collections get upload/download/preview affordances in the view.
  def _load_collection_type
    result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    collections = (result["data"] ?? {})["collections"] ?? []
    matching = collections.filter do |coll| coll["name"] == @collection_name end
    @collection_type = matching.length() > 0 ? (matching[0]["type"] ?? "document") : "document"
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
    payload = { "query": query, "bindVars": { "offset": @offset, "batch": @limit + 1 } }
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
