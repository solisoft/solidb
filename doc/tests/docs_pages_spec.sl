# Renders the documentation pages that describe behaviour the server actually
# has. A page that fails to render, or loses the paragraph that documents a
# feature, fails here instead of shipping silently.

describe("Docs pages") do
  test("the home page carries the version being released") do
    response = get("/")
    expect(res_status(response)).to_equal(200)
    expect(res_body(response).include?("v2.2.1")).to_equal(true)
  end

  test("a docs page shows the same version as the landing page") do
    marker = "ver-pill\">"
    home_body = res_body(get("/"))
    docs_body = res_body(get("/docs/offline-sync"))
    expect(home_body.include?(marker)).to_equal(true)
    expect(docs_body.include?(marker)).to_equal(true)

    # split, not index arithmetic: length() counts bytes and index_of() characters,
    # and these pages contain multibyte text before the pill.
    home_version = home_body.split(marker)[1].split("<")[0]
    docs_version = docs_body.split(marker)[1].split("<")[0]

    expect(home_version.starts_with("v")).to_equal(true)
    expect(docs_version).to_equal(home_version)
  end

  test("the changelog has a section for the release") do
    response = get("/docs/changelog")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("v2.2.0")).to_equal(true)
    expect(body.include?("RocksDB 11.8.1")).to_equal(true)
    expect(body.include?("Minimum Rust 1.91")).to_equal(true)
  end

  test("transactions documents what a writing query can and cannot do") do
    response = get("/docs/transactions")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("mutationCount")).to_equal(true)
    expect(body.include?("OPTIONS and REPLACE")).to_equal(true)
    expect(body.include?("Read-your-writes")).to_equal(true)
  end

  test("the driver page lists the commands that used to be refused") do
    response = get("/docs/driver")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("prune_collection")).to_equal(true)
    expect(body.include?("repair_collection")).to_equal(true)
    expect(body.include?("transaction_command")).to_equal(true)
    expect(body.include?("geo_within")).to_equal(true)
    expect(body.include?("aggregate_columnar")).to_equal(true)
  end

  test("triggers document the filter field") do
    response = get("/docs/triggers")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("A filter that raises an error does not fire")).to_equal(true)
  end

  test("sharding documents writes and the replace acknowledgement") do
    response = get("/docs/sharding")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("x-replace-applied")).to_equal(true)
    expect(body.include?("not atomic against a concurrent writer")).to_equal(true)
  end

  test("offline sync documents conflict detection and resolution") do
    response = get("/docs/offline-sync")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("What the server does on push")).to_equal(true)
    expect(body.include?("/_api/sync/conflicts")).to_equal(true)
    expect(body.include?("/_api/sync/resolve")).to_equal(true)
    expect(body.include?("delta_patch")).to_equal(true)
  end

  test("tooling documents SQL restore") do
    response = get("/docs/tooling")
    expect(res_status(response)).to_equal(200)
    expect(res_body(response).include?("SQL dumps.")).to_equal(true)
  end

  test("columnar documents the driver filter") do
    response = get("/docs/columnar")
    expect(res_status(response)).to_equal(200)
    expect(res_body(response).include?("aggregate_columnar")).to_equal(true)
  end

  test("the auth API documents database-limited role assignment") do
    response = get("/docs/api-auth")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("role@database")).to_equal(true)
    expect(body.include?("cannot contain")).to_equal(true)
  end

  test("an unknown docs page is a 404, not a template lookup") do
    response = get("/docs/not-a-real-page")
    expect(res_status(response)).to_equal(404)
  end
end
