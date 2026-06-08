# Documents browser against a scratch database + collection seeded per suite.
describe("DocumentsController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_docs" })
    SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_docs"), { "name": "people" })
    SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_docs", "people"),
                          { "_key": "alice", "name": "Alice", "age": 30 })
    SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_docs", "people"),
                          { "_key": "bob", "name": "Bob", "age": 20 })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_docs"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/collections/:name/docs") do
    test("lists the documents") do
      response = get("/databases/admin_spec_docs/collections/people/docs")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "alice")
      assert_contains(body, "bob")
    end

    test("filters with an sdbql expression") do
      response = get("/databases/admin_spec_docs/collections/people/docs?filter=" + url("doc.age > 25"))
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "alice")
      assert_not(body.includes?("\"bob\""))
      assert_contains(body, "filtered")
    end

    test("quoted filter round-trips without double escaping") do
      response = get("/databases/admin_spec_docs/collections/people/docs?filter=" +
                     url("doc.name == \"Alice\""))
      assert_eq(res_status(response), 200)
      body = res_body(response)
      # <%- attr(@filter) %>: attr-escaped exactly once. <%= attr(...) %> would
      # re-escape the & and the input would display literal &quot;.
      assert_contains(body, "value=\"doc.name == &quot;Alice&quot;\"")
      assert_not(body.includes?("&amp;quot;"))
      assert_contains(body, "documents 1–1")
    end

    test("surfaces filter errors") do
      response = get("/databases/admin_spec_docs/collections/people/docs?filter=" + url("doc.,,,"))
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "filter error")
    end

    test("edit modal textarea excludes system attributes") do
      response = get("/databases/admin_spec_docs/collections/people/docs")
      body = res_body(response)
      # The editable JSON lives in the x-ref="docEditor" textarea; system
      # attributes move to the modal header (read-only).
      segments = body.split("x-ref=\"docEditor\"")
      assert_gt(segments.length(), 1)
      editable_json = segments[1].split("</textarea>")[0]
      assert_contains(editable_json, "name")
      assert_not(editable_json.includes?("_rev"))
      assert_not(editable_json.includes?("_created_at"))
      assert_not(editable_json.includes?("_key"))
    end

    test("paginates with offset and limit") do
      response = get("/databases/admin_spec_docs/collections/people/docs?limit=25&offset=1")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "documents 2–2")
    end
  end

  describe("document lifecycle") do
    test("create, update, delete") do
      response = post("/databases/admin_spec_docs/collections/people/docs",
                      { "document": "{\"_key\": \"carol\", \"name\": \"Carol\", \"age\": 41}" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "document created")

      response = put("/databases/admin_spec_docs/collections/people/docs/carol",
                     { "document": "{\"name\": \"Carol\", \"age\": 42}" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "document carol updated")

      response = delete("/databases/admin_spec_docs/collections/people/docs/carol")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "document carol deleted")
    end

    test("rejects invalid document json") do
      response = post("/databases/admin_spec_docs/collections/people/docs", { "document": "{nope" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "document must be a JSON object")
    end
  end

  describe("export / import") do
    test("export streams jsonl with attachment headers") do
      response = get("/databases/admin_spec_docs/collections/people/docs/export")
      assert_eq(res_status(response), 200)
      assert_contains(res_header(response, "Content-Type"), "x-ndjson")
      assert_contains(res_header(response, "Content-Disposition"), "admin_spec_docs-people.jsonl")
      body = res_body(response)
      assert_contains(body, "alice")
      assert_contains(body, "bob")
      assert_contains(body, "_collectionType")
    end

    test("import a jsonl file") do
      boundary = "----specboundary10"
      content = "{\"_key\": \"imp1\", \"name\": \"Imp One\"}\n{\"_key\": \"imp2\", \"name\": \"Imp Two\"}\n"
      body = "--" + boundary + "\r\n"
      body = body + "Content-Disposition: form-data; name=\"file\"; filename=\"docs.jsonl\"\r\n"
      body = body + "Content-Type: application/octet-stream\r\n\r\n"
      body = body + content + "\r\n"
      body = body + "--" + boundary + "--\r\n"
      response = request("POST", "/databases/admin_spec_docs/collections/people/docs/import", body,
                         { "headers": { "Content-Type": "multipart/form-data; boundary=" + boundary } })
      assert_eq(res_status(response), 200)
      page = res_body(response)
      assert_contains(page, "imported 2 document(s)")
      assert_contains(page, "imp1")
      assert_contains(page, "imp2")
    end

    test("import a pretty-printed json array") do
      boundary = "----specboundary11"
      content = "[\n  { \"_key\": \"imp3\", \"name\": \"Imp Three\" },\n" +
                "  { \"_key\": \"imp4\", \"name\": \"Imp Four\" }\n]\n"
      body = "--" + boundary + "\r\n"
      body = body + "Content-Disposition: form-data; name=\"file\"; filename=\"docs.json\"\r\n"
      body = body + "Content-Type: application/json\r\n\r\n"
      body = body + content + "\r\n"
      body = body + "--" + boundary + "--\r\n"
      response = request("POST", "/databases/admin_spec_docs/collections/people/docs/import", body,
                         { "headers": { "Content-Type": "multipart/form-data; boundary=" + boundary } })
      assert_eq(res_status(response), 200)
      page = res_body(response)
      assert_contains(page, "imported 2 document(s)")
      assert_contains(page, "imp3")
      assert_contains(page, "imp4")
    end

    test("import without a file is rejected") do
      response = post("/databases/admin_spec_docs/collections/people/docs/import", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "no file selected")
    end
  end

  describe("truncate") do
    test("truncates the collection from the docs page") do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_docs"), { "name": "wipe_me" })
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_docs", "wipe_me"), { "_key": "gone", "x": 1 })

      response = put("/databases/admin_spec_docs/collections/wipe_me/docs/truncate", {})
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "collection wipe_me truncated")
      assert_contains(body, "no documents")

      SolidbClient.delete_api(SolidbEndpoints.collection("admin_spec_docs", "wipe_me"))
    end
  end

  describe("json schema") do
    test("set, render in the editor modal, remove") do
      schema_json = "{\"type\": \"object\", \"properties\": {\"name\": {\"type\": \"string\"}}}"
      response = put("/databases/admin_spec_docs/collections/people/docs/schema",
                     { "schema": schema_json, "validation_mode": "strict" })
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "schema updated (strict)")
      # Header button badge reflects the active mode.
      assert_contains(body, "· strict")
      # The editor textarea is pre-filled with the stored schema.
      segments = body.split("x-ref=\"schemaEditor\"")
      assert_gt(segments.length(), 1)
      assert_contains(segments[1].split("</textarea>")[0], "object")

      response = delete("/databases/admin_spec_docs/collections/people/docs/schema")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "schema removed")
    end

    test("rejects invalid schema json") do
      response = put("/databases/admin_spec_docs/collections/people/docs/schema",
                     { "schema": "{nope", "validation_mode": "strict" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "schema must be a valid JSON object")
    end

    test("rejects a schema that is not a json object") do
      response = put("/databases/admin_spec_docs/collections/people/docs/schema",
                     { "schema": "[1, 2]", "validation_mode": "strict" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "schema must be a valid JSON object")
    end

    test("unknown validation mode falls back to off") do
      response = put("/databases/admin_spec_docs/collections/people/docs/schema",
                     { "schema": "{\"type\": \"object\"}", "validation_mode": "bogus" })
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "schema updated (off)")
      delete("/databases/admin_spec_docs/collections/people/docs/schema")
    end
  end

  describe("blob collections") do
    before_all() do
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_docs"), { "name": "files", "type": "blob" })
    end

    test("upload via multipart, browse, download, delete") do
      boundary = "----specboundary7"
      file_content = "hello blob spec content"
      body = "--" + boundary + "\r\n"
      body = body + "Content-Disposition: form-data; name=\"file\"; filename=\"spec.txt\"\r\n"
      body = body + "Content-Type: text/plain\r\n\r\n"
      body = body + file_content + "\r\n"
      body = body + "--" + boundary + "--\r\n"
      response = request("POST", "/databases/admin_spec_docs/collections/files/docs/upload", body,
                         { "headers": { "Content-Type": "multipart/form-data; boundary=" + boundary } })
      assert_eq(res_status(response), 200)
      page = res_body(response)
      assert_contains(page, "file spec.txt uploaded")
      assert_contains(page, "spec.txt")
      assert_contains(page, "Download")

      conn = Solidb(SolidbClient.host(), "admin_spec_docs")
      conn.auth(SolidbClient.username(), SolidbClient.password())
      listing = conn.query("FOR b IN files RETURN b._key")
      assert_gt(listing.length(), 0)
      blob_key = listing[0]

      response = get("/databases/admin_spec_docs/collections/files/docs/" + blob_key + "/blob")
      assert_eq(res_status(response), 200)
      assert_eq(res_body(response), file_content)
      assert_contains(res_header(response, "Content-Disposition"), "inline")

      response = get("/databases/admin_spec_docs/collections/files/docs/" + blob_key + "/blob?download=1")
      assert_contains(res_header(response, "Content-Disposition"), "attachment")
      assert_contains(res_header(response, "Content-Disposition"), "spec.txt")

      response = delete("/databases/admin_spec_docs/collections/files/docs/" + blob_key)
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "document " + blob_key + " deleted")
    end

    test("upload without a file is rejected") do
      response = post("/databases/admin_spec_docs/collections/files/docs/upload", {})
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "no file selected")
    end

    test("blob collections cannot be exported as jsonl") do
      response = get("/databases/admin_spec_docs/collections/files/docs/export")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "blob collections cannot be exported")
    end

    test("drag-drop JSON branch: upload + error both answer JSON") do
      boundary = "----specboundary8"
      body = "--" + boundary + "\r\n"
      body = body + "Content-Disposition: form-data; name=\"file\"; filename=\"drop.txt\"\r\n"
      body = body + "Content-Type: text/plain\r\n\r\ndropped\r\n"
      body = body + "--" + boundary + "--\r\n"
      response = request("POST", "/databases/admin_spec_docs/collections/files/docs/upload", body,
                         { "headers": { "Content-Type": "multipart/form-data; boundary=" + boundary,
                                        "Accept": "application/json" } })
      assert_eq(res_status(response), 200)
      data = res_json(response)
      assert(data["ok"])
      assert_not(data["blob_id"].blank?)
      delete("/databases/admin_spec_docs/collections/files/docs/" + data["blob_id"])

      response = request("POST", "/databases/admin_spec_docs/collections/files/docs/upload", "",
                         { "headers": { "Accept": "application/json" } })
      data = res_json(response)
      assert_not(data["ok"])
      assert_eq(data["error"], "no file selected")
    end

    test("missing blob returns 404") do
      response = get("/databases/admin_spec_docs/collections/files/docs/nope/blob")
      assert_eq(res_status(response), 404)
    end
  end
end
