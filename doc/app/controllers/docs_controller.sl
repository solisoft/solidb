# Docs controller — serves the SoliDB documentation migrated from the old www
# site. Each page is a static view at app/views/docs/<slug>.html.slv, rendered
# inside the "docs" layout (top nav + sidebar). `show` only renders slugs on the
# allowlist, so a bad :page param can't reach an arbitrary template.

class DocsController < Controller
    # GET /docs
    def index
        render("docs/index", { "title": "SoliDB Documentation", "current_page": "index", "layout": "docs" })
    end

    # GET /docs/:page
    def show
        page = params["page"].to_s

        # Support nested paths like getting-started/changelog by normalizing
        if page.include?("/") then
            page = page.gsub("/", "-")
        end

        return this.not_found() unless this.valid_pages().includes?(page)

        title = this.titles()[page] ?? this.titleize(page)

        # Map getting-started-changelog to the changelog view file
        view_name = page == "getting-started-changelog" ? "changelog" : page
        current = page

        render("docs/#{view_name}", { "title": "#{title} — SoliDB Docs", "current_page": current, "layout": "docs" })
    end

    # --- helpers -------------------------------------------------------------

    # Turn "vector-search" into "Vector Search" for pages without a curated title.
    def titleize(slug)
        words = slug.split("-").map do |word|
            if word.length() == 0
                word
            else
                word.substring(0, 1).upcase() + word.substring(1, word.length())
            end
        end
        return words.join(" ")
    end

    def not_found
        body = "<!doctype html><meta charset=\"utf-8\"><title>Not found</title>" +
            "<body style=\"background:#090c0b;color:#ece6d6;font-family:system-ui;" +
            "display:flex;min-height:100vh;align-items:center;justify-content:center;text-align:center\">" +
            "<div><h1 style=\"font-size:2rem;margin-bottom:8px\">404</h1>" +
            "<p style=\"color:#9ba39c\">That documentation page doesn't exist. " +
            "<a href=\"/docs\" style=\"color:#f5a623\">Back to the docs</a>.</p></div>"
        return { "status": 404, "headers": { "Content-Type": "text/html; charset=utf-8" }, "body": body }
    end

    # Curated titles (everything else falls back to titleize()).
    def titles
        return {
            "getting-started": "Getting Started",
            "sdbql": "SDBQL Reference",
            "sql": "SQL Compatibility",
            "api": "API Reference",
            "driver": "Native Driver",
            "nl-queries": "Natural Language Queries",
            "transactions": "ACID & Distributed Transactions",
            "vector-search": "Vector Search",
            "hybrid-search": "Hybrid Search",
            "graphs": "Graphs & Edges",
            "graph-rag": "Graph RAG",
            "views": "Materialized Views",
            "streams": "Stream Processing",
            "scripting": "Lua Scripting",
            "live-queries": "Live Queries",
            "documents": "Documents Storage",
            "timeseries": "Time Series",
            "columnar": "Columnar Storage",
            "clients": "Official Clients",
            "clients-nodejs": "Node.js / Bun Client",
            "comparison": "Database Comparison",
            "tooling": "Command-Line Tools",
            "offline-sync": "Offline Sync",
            "changelog": "Changelog",
            "getting-started-changelog": "Changelog"
        }
    end

    def valid_pages
        return ["index", "api", "api-auth", "api-blobs", "api-cluster", "api-collections", "api-databases",
            "api-documents", "api-indexes", "api-monitoring", "api-queries",
            "api-scripting", "api-transactions", "api-triggers", "api-vector", "architecture",
            "blobs", "changefeeds", "clients", "clients-android", "clients-elixir",
            "clients-flutter", "clients-go", "clients-ios", "clients-laravel",
            "clients-nodejs", "clients-php", "clients-python", "clients-react-native",
            "clients-ruby", "clients-rust", "cluster", "columnar", "comparison", "documents",
            "driver", "getting-started", "graph-rag", "graphs", "hybrid-search", "indexes",
            "live-queries", "nl-queries", "observability", "offline-sync",
            "scripting", "scripting-core", "scripting-database", "scripting-development",
            "scripting-files", "scripting-management", "scripting-services", "scripting-streams",
            "scripting-tutorial-crud", "scripting-tutorial-testing", "scripting-utils",
            "scripting-validation", "scripting-ws", "sdbql", "sdbql-aggregations",
            "sdbql-benchmarks", "sdbql-cte", "sdbql-functions-array", "sdbql-functions-crypto",
            "sdbql-functions-date", "sdbql-functions-geo", "sdbql-functions-misc",
            "sdbql-functions-numeric", "sdbql-functions-search", "sdbql-functions-string",
            "sdbql-functions-vector", "sdbql-graphs", "sdbql-mutations", "sdbql-operators",
            "sdbql-syntax", "security", "sharding", "sql", "streams", "timeseries", "tooling",
            "transactions", "triggers", "vector-search", "views", "changelog", "getting-started-changelog"]
    end
end
