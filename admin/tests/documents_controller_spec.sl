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

    test("surfaces filter errors") do
      response = get("/databases/admin_spec_docs/collections/people/docs?filter=" + url("doc.,,,"))
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "filter error")
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
