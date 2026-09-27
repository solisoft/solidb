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
  })
})
