# The site's 404 page (app/views/errors/404.html.slv): what an unmatched route
# gets, and what DocsController and BlogController render for a slug they
# don't have.

describe("Not found page") do
  test("an unmatched route gets the site's 404 page") do
    response = get("/no/such/place")
    expect(res_status(response)).to_equal(404)
    body = res_body(response)
    expect(body.include?("<title>Page not found — SoliDB</title>")).to_equal(true)
    expect(body.include?("No document at this address.")).to_equal(true)
    expect(body.include?("name=\"robots\" content=\"noindex\"")).to_equal(true)
    expect(body.include?("Back to the home page")).to_equal(true)
  end

  test("an unknown docs page renders it with a link back to the docs") do
    response = get("/docs/sdbql-function-date")
    expect(res_status(response)).to_equal(404)
    body = res_body(response)
    expect(body.include?("There is no documentation page at this address.")).to_equal(true)
    expect(body.include?("href=\"/docs\">Back to the docs</a>")).to_equal(true)
    # The address it was asked for, in the query card.
    expect(body.include?("\"/docs/sdbql-function-date\"")).to_equal(true)
  end

  test("it carries every docs page for Did you mean") do
    body = res_body(get("/docs/vectr-search"))
    expect(body.include?("{\"path\":\"/docs/vector-search\",\"label\":\"Vector Search\"}")).to_equal(true)
    expect(body.include?("\"label\":\"SDBQL Reference: Date Functions\"")).to_equal(true)
  end

  test("an unknown post renders it with the posts as candidates") do
    response = get("/blog/solidb-2-1-transactions")
    expect(res_status(response)).to_equal(404)
    body = res_body(response)
    expect(body.include?("There is no post at this address.")).to_equal(true)
    expect(body.include?("Back to the blog")).to_equal(true)
    expect(body.include?("\"path\":\"/blog/solidb-2-1-transactions-conflicts-roles\"")).to_equal(true)
  end

  test("a docs 404 does not carry the blog's posts") do
    body = res_body(get("/docs/nope"))
    expect(body.include?("\"path\":\"/blog/solidb-2-1-transactions-conflicts-roles\"")).to_equal(false)
  end
end
