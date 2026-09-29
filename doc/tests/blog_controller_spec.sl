describe("BlogController", fn() {
  test("GET /blog lists the posts", fn() {
    response = get("/blog")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("Release notes and deep dives")).to_equal(true)
    expect(body.include?("/blog/sdbql-1-3-new-functions")).to_equal(true)
  })

  test("GET /blog/:slug renders a known post", fn() {
    response = get("/blog/sdbql-1-3-new-functions")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.include?("33 new SDBQL functions")).to_equal(true)
    expect(body.include?("DATE_SERIES")).to_equal(true)
    expect(body.include?("application/atom+xml")).to_equal(true)
  })

  test("every listed post renders with its title", fn() {
    posts = {
      "solidb-2-1-transactions-conflicts-roles": "A driver transaction that was not one",
      "solidb-2-0-shared-keyspace": "creating a collection no longer depends",
      "a-request-must-never-take-the-server-down": "20,200 rows",
      "faster-startup-many-collections": "What hundreds of collections cost",
      "where-the-memory-goes": "jemalloc now serves RocksDB",
      "secure-by-default": "the collections you can no longer write by name",
      "solidb-1-0": "SoliDB 1.0: closed by default",
      "clustering-across-machines": "a cluster that actually spans machines",
      "columnar-collections-in-sdbql": "Columnar collections now take FILTER",
      "backups-checkpoints-vs-dumps": "Backing up SoliDB"
    }
    posts.each(&{ |slug, title|
      response = get("/blog/#{slug}")
      expect(res_status(response)).to_equal(200)
      expect(res_body(response).include?(title)).to_equal(true)
    })
  })

  test("the index links every post, newest first", fn() {
    body = res_body(get("/blog"))
    expect(body.split("class=\"post-card\"").length()).to_equal(12)
    newest = body.index_of("/blog/solidb-2-1-transactions-conflicts-roles")
    oldest = body.index_of("/blog/backups-checkpoints-vs-dumps")
    expect(newest > 0 && newest < oldest).to_equal(true)
  })

  test("GET /blog/:slug is 404 for an unknown slug", fn() {
    response = get("/blog/no-such-post")
    expect(res_status(response)).to_equal(404)
    expect(res_body(response).include?("Back to the blog")).to_equal(true)
  })

  test("GET /blog/:slug does not render arbitrary templates", fn() {
    response = get("/blog/index")
    expect(res_status(response)).to_equal(404)
  })

  test("GET /blog/feed.xml is an Atom feed of the posts", fn() {
    response = get("/blog/feed.xml")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(body.starts_with("<?xml")).to_equal(true)
    expect(body.include?("<feed xmlns=\"http://www.w3.org/2005/Atom\">")).to_equal(true)
    expect(body.include?("/blog/sdbql-1-3-new-functions</id>")).to_equal(true)
    expect(body.include?("/blog/solidb-2-1-transactions-conflicts-roles</id>")).to_equal(true)
  })
})
