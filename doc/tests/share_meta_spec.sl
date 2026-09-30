# The <title>, description and Open Graph tags each page is shared with.
# Docs pages derive theirs from the sidebar and the page's own lead
# (app/helpers/docs_helper.sl); blog posts from BlogController#posts.

# The content="" of the first <meta> whose property or name is `key`.
def share_meta(body, key)
  marker = "=\"" + key + "\" content=\""
  parts = body.split(marker)
  return nil if parts.length < 2

  parts[1].split("\"")[0]
end

def share_title_tag(body)
  body.split("<title>")[1].split("</title>")[0]
end

describe("Share metadata") do
  test("a docs subpage is titled by its sidebar group and label") do
    body = res_body(get("/docs/sdbql-functions-date"))
    expect(share_title_tag(body)).to_equal("SDBQL Reference: Date Functions — SoliDB Docs")
    expect(share_meta(body, "og:title")).to_equal("SDBQL Reference: Date Functions — SoliDB Docs")
    expect(share_meta(body, "og:type")).to_equal("article")
  end

  test("a docs page is described by its own lead paragraph") do
    body = res_body(get("/docs/vector-search"))
    description = share_meta(body, "og:description")
    expect(description.starts_with("Enable AI-powered semantic search")).to_equal(true)
    expect(share_meta(body, "description")).to_equal(description)
    expect(share_meta(body, "twitter:description")).to_equal(description)
  end

  test("an API page without a lead is described by its endpoints") do
    body = res_body(get("/docs/api-databases"))
    description = share_meta(body, "og:description")
    expect(description.starts_with("Databases — 3 SoliDB HTTP API endpoints.")).to_equal(true)
    expect(description.include?("Create a new database.")).to_equal(true)
  end

  test("a client page without a lead is described by its sections") do
    description = share_meta(res_body(get("/docs/clients-go")), "og:description")
    expect(description.starts_with("Go Client — Getting Started, Core Operations")).to_equal(true)
  end

  test("a long lead is cut on a word, with an ellipsis, within 160 characters") do
    description = share_meta(res_body(get("/docs/sdbql")), "og:description")
    expect(description.ends_with("…")).to_equal(true)
    expect(description.chars().length <= 160).to_equal(true)
  end

  test("no sidebar page falls back to the site-wide description") do
    fallback = "The SoliDB documentation — SDBQL, the HTTP API"
    slugs = ["api", "api-auth", "api-monitoring", "changelog", "clients", "clients-rust",
      "graph-rag", "scripting-utils", "sdbql-cte", "sdbql-operators", "timeseries", "tooling"]
    slugs.each do |slug|
      description = share_meta(res_body(get("/docs/" + slug)), "og:description")
      expect(description.blank?).to_equal(false)
      expect(description.starts_with(fallback)).to_equal(false)
    end
  end

  test("a page_title in the nav replaces the qualified label") do
    body = res_body(get("/docs/changelog"))
    expect(share_title_tag(body)).to_equal("Changelog — SoliDB Docs")
  end

  test("the docs home is a website, titled for the whole documentation") do
    body = res_body(get("/docs"))
    expect(share_title_tag(body)).to_equal("SoliDB Documentation")
    expect(share_meta(body, "og:type")).to_equal("website")
  end

  test("a blog post shares under its own headline, dated and tagged") do
    body = res_body(get("/blog/solidb-2-0-shared-keyspace"))
    headline = "SoliDB 2.0: creating a collection no longer depends on how many you have"
    expect(share_meta(body, "og:title")).to_equal(headline)
    expect(share_title_tag(body)).to_equal(headline + " — SoliDB Blog")
    expect(share_meta(body, "og:type")).to_equal("article")
    expect(share_meta(body, "article:published_time")).to_equal("2026-09-28T00:00:00Z")
    expect(body.include?("property=\"article:tag\" content=\"Storage\"")).to_equal(true)
    expect(share_meta(body, "og:description").starts_with("Every collection used to be")).to_equal(true)
  end

  test("the blog index is a website that names the newest post") do
    response = get("/blog")
    expect(res_status(response)).to_equal(200)
    body = res_body(response)
    expect(share_meta(body, "og:type")).to_equal("website")
    expect(share_meta(body, "og:description").include?("Latest: ")).to_equal(true)
    expect(body.include?("article:published_time")).to_equal(false)
  end
end
