# Blog controller — release notes and deep dives, written as static views.
# Each post is app/views/blog/<slug>.html.slv, listed in posts() newest first.
# `show` only renders slugs from that list, so a bad :slug can't reach an
# arbitrary template. Adding a post = one view + one entry in posts().

class BlogController < Controller
  # GET /blog
  def index
    posts = this.posts()
    render(
      "blog/index",
      {
        "title": "Blog — SoliDB",
        # Names the newest post, so a shared link to /blog shows what is new.
        "description": "Release notes and engineering deep dives from the SoliDB team. "
        + "Latest: #{posts[0]["title"]}",
        "posts": posts,
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
        # The layout's share tags read the post as `article`: in a view, an
        # unset `post` is not nil but the router's post() function.
        "article": post,
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
    return [
      {
        "slug": "solidb-2-1-transactions-conflicts-roles",
        "title": "SoliDB 2.1: transactions that mean it, sync conflicts you can see, roles that stop at a database",
        "date": "2026-09-29",
        "display_date": "September 29, 2026",
        "summary": "A driver transaction that committed on the spot, two writes of one unique value "
        + "that both passed, and a custom role that granted nothing: what 2.1.0 fixes, plus "
        + "database-limited roles, sync conflict resolution and OPTIONS on sharded collections.",
        "tags": ["Correctness", "Release 2.1.0"],
        "read_minutes": 8
      },
      {
        "slug": "solidb-2-0-shared-keyspace",
        "title": "SoliDB 2.0: creating a collection no longer depends on how many you have",
        "date": "2026-09-28",
        "display_date": "September 28, 2026",
        "summary": "Every collection used to be a RocksDB column family, and every create rewrote "
        + "a file sized by the whole instance. 2.0 moves them into one shared keyspace: "
        + "10 creates went from 1,952 ms to 53 ms.",
        "tags": ["Storage", "Release 2.0.0"],
        "read_minutes": 7
      },
      {
        "slug": "sdbql-1-3-new-functions",
        "title": "33 new SDBQL functions, and FOR x IN anything",
        "date": "2026-09-27",
        "display_date": "September 27, 2026",
        "summary": "SoliDB 1.2.2 and 1.3.0 add date series, lookup maps, object diffs, "
        + "accent folding, IBAN and SIRET checks, banker's rounding — and FOR now "
        + "accepts any expression after IN.",
        "tags": ["SDBQL", "Release 1.3.0"],
        "read_minutes": 9
      },
      {
        "slug": "a-request-must-never-take-the-server-down",
        "title": "A 3 MB bind variable, 20,200 rows, and a server killed at 61 GB",
        "date": "2026-09-26",
        "display_date": "September 26, 2026",
        "summary": "SoliDB's rule is that a request may fail but never take the server down. A bulk "
        + "UPSERT over @rows broke it: bind variables were copied once per row. How 1.2.3 "
        + "fixed it.",
        "tags": ["Reliability", "Release 1.2.3"],
        "read_minutes": 6
      },
      {
        "slug": "faster-startup-many-collections",
        "title": "What hundreds of collections cost, and what 1.2.0 took off the bill",
        "date": "2026-09-24",
        "display_date": "September 24, 2026",
        "summary": "One collection is one RocksDB column family, and each create or drop rewrites "
        + "the whole OPTIONS file. 1.2.0 stops paying for that at startup, on delete-and-"
        + "recreate, on listing, and on every WAL flush.",
        "tags": ["Performance", "Release 1.2.0"],
        "read_minutes": 7
      },
      {
        "slug": "where-the-memory-goes",
        "title": "jemalloc now serves RocksDB, and /metrics says where memory goes",
        "date": "2026-09-03",
        "display_date": "September 3, 2026",
        "summary": "Until 1.1.0, RocksDB's block cache, table readers and memtables lived in glibc "
        + "arenas that jemalloc's tuning never reached. What changed, the new per-component"
        + " gauges in /metrics, and the knobs that bound memory.",
        "tags": ["Internals", "Release 1.1.0"],
        "read_minutes": 7
      },
      {
        "slug": "secure-by-default",
        "title": "SoliDB 1.1.0: the collections you can no longer write by name",
        "date": "2026-09-03",
        "display_date": "September 3, 2026",
        "summary": "1.1.0 puts SoliDB's own collections in three tiers behind one check, makes Lua "
        + "scripts write as their caller, and gates the admin console. Each one closes a "
        + "real finding; here is what a Write user now sees.",
        "tags": ["Security", "Release 1.1.0"],
        "read_minutes": 7
      },
      {
        "slug": "solidb-1-0",
        "title": "SoliDB 1.0: closed by default, set operations, and no more job queue",
        "date": "2026-08-31",
        "display_date": "August 31, 2026",
        "summary": "1.0.0 starts a fresh node closed (loopback, keyfile-only replication, "
        + "authenticated /metrics), adds UNION/INTERSECT/EXCEPT, recursive CTEs and RETURN "
        + "DISTINCT, and moves background jobs to Soli.",
        "tags": ["SDBQL", "Release 1.0.0"],
        "read_minutes": 8
      },
      {
        "slug": "clustering-across-machines",
        "title": "SoliDB 0.34.0: a cluster that actually spans machines",
        "date": "2026-08-05",
        "display_date": "August 5, 2026",
        "summary": "Before 0.34.0 a multi-machine cluster came up and replicated nothing. What was "
        + "broken, --host vs --advertise, the now-mandatory keyfile, and a three-node setup"
        + " on three machines.",
        "tags": ["Cluster", "Release 0.34.0"],
        "read_minutes": 9
      },
      {
        "slug": "columnar-collections-in-sdbql",
        "title": "Columnar collections now take FILTER, SORT and joins in SDBQL",
        "date": "2026-07-27",
        "display_date": "July 27, 2026",
        "summary": "In 0.33.0, FOR reads columnar collections like any other source, so FILTER, "
        + "SORT, joins and subqueries work on them. Four aggregate bugs that returned wrong"
        + " numbers instead of errors are also fixed.",
        "tags": ["SDBQL", "Release 0.33.0"],
        "read_minutes": 7
      },
      {
        "slug": "backups-checkpoints-vs-dumps",
        "title": "Backing up SoliDB: checkpoints, dumps, and when to use each",
        "date": "2026-07-27",
        "display_date": "July 27, 2026",
        "summary": "0.33.0 adds POST /_api/backup, an instant RocksDB checkpoint of the whole "
        + "instance, next to solidb-dump's portable JSONL. What each does, what it can't "
        + "do, a decision table, and a runbook.",
        "tags": ["Operations", "Release 0.33.0"],
        "read_minutes": 7
      }
    ]
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

  # The site's 404 page (app/views/errors/404.html.slv), which is also what
  # an unmatched route gets.
  def not_found
    return render("errors/404", {
      "layout": false,
      "status": 404,
      "message": "There is no post at this address.",
      "back_href": "/blog",
      "back_label": "Back to the blog",
      "extra_candidates": this.posts().map do |post|
        {"path": "/blog/" + post["slug"], "label": post["title"]}
      end
    }, {"status": 404})
  end
end
