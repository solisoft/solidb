# Blog controller — release notes and deep dives, written as static views.
# Each post is app/views/blog/<slug>.html.slv, listed in posts() newest first.
# `show` only renders slugs from that list, so a bad :slug can't reach an
# arbitrary template. Adding a post = one view + one entry in posts().

class BlogController < Controller
  # GET /blog
  def index
    render(
      "blog/index",
      {
        "title": "Blog — SoliDB",
        "description": "Release notes and deep dives from the SoliDB team.",
        "posts": this.posts(),
        "layout": "blog"
      }
    )
  end

  # GET /blog/:slug
  def show
    slug = params["slug"].to_s
    post = this.find_post(slug)
    return this.not_found() if post.nil?

    render(
      "blog/#{slug}",
      {
        "title": "#{post["title"]} — SoliDB Blog",
        "description": post["summary"],
        "post": post,
        "layout": "blog"
      }
    )
  end

  # GET /blog/feed.xml — Atom feed of every post.
  def feed
    base = this._site_origin() + "/blog"
    entries = this.posts().map do |post|
      url = "#{base}/#{post["slug"]}"
      "<entry><title>#{this.xml_escape(post["title"])}</title>" + "<link href=\"#{url}\"/><id>#{url}</id>"
      + "<updated>#{post["date"]}T00:00:00Z</updated>"
      + "<summary>#{this.xml_escape(post["summary"])}</summary></entry>"
    end
    latest = this.posts()[0]["date"]
    body = "<?xml version=\"1.0\" encoding=\"utf-8\"?>" + "<feed xmlns=\"http://www.w3.org/2005/Atom\">"
    + "<title>SoliDB Blog</title><link href=\"#{base}\"/>"
    + "<link rel=\"self\" href=\"#{base}/feed.xml\"/><id>#{base}</id>"
    + "<updated>#{latest}T00:00:00Z</updated>"
    + entries.join("")
    + "</feed>"
    return {
      "status": 200,
      "headers": {"Content-Type": "application/atom+xml; charset=utf-8"},
      "body": body
    }
  end

  # --- helpers -------------------------------------------------------------

  # Newest first. `date` is ISO (YYYY-MM-DD); `display_date` is what readers see.
  def posts
    return [{
      "slug": "sdbql-1-3-new-functions",
      "title": "33 new SDBQL functions, and FOR x IN anything",
      "date": "2026-09-27",
      "display_date": "September 27, 2026",
      "summary": "SoliDB 1.2.2 and 1.3.0 add date series, lookup maps, object diffs, "
      + "accent folding, IBAN and SIRET checks, banker's rounding — and FOR now "
      + "accepts any expression after IN.",
      "tags": ["SDBQL", "Release 1.3.0"],
      "read_minutes": 9
    }]
  end

  # Same origin as the site_url() view helper (app/helpers/application_helper.sl),
  # which controllers can't call: feed readers need absolute links.
  def _site_origin
    origin = getenv("SITE_URL").to_s
    origin = "https://solidb.solisoft.net" if origin.blank?
    return origin.ends_with("/") ? origin.substring(0, len(origin) - 1) : origin
  end

  def find_post(slug)
    return this.posts().find do |post|
      post["slug"] == slug
    end
  end

  def xml_escape(text)
    return text.gsub("&", "&amp;").gsub("<", "&lt;").gsub(">", "&gt;").gsub("\"", "&quot;")
  end

  def not_found
    body = "<!doctype html><meta charset=\"utf-8\"><title>Not found</title>"
    + "<body style=\"background:#090c0b;color:#ece6d6;font-family:system-ui;"
    + "display:flex;min-height:100vh;align-items:center;justify-content:center;text-align:center\">"
    + "<div><h1 style=\"font-size:2rem;margin-bottom:8px\">404</h1>"
    + "<p style=\"color:#9ba39c\">That post doesn't exist. "
    + "<a href=\"/blog\" style=\"color:#f5a623\">Back to the blog</a>.</p></div>"
    return {
      "status": 404,
      "headers": {"Content-Type": "text/html; charset=utf-8"},
      "body": body
    }
  end
end
