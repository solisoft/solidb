# Docs navigation and the per-page share metadata derived from it.
#
# docs_nav() is the sidebar's data (app/views/docs/_sidebar.html.slv renders
# it). docs_meta() reads the same data, so a page's share title is the label
# readers see in the sidebar, and its description is the page's own lead
# paragraph — nothing to keep in step by hand when a page is added or edited.

def docs_nav()
    return [
        { "title": "Getting Started", "items": [
          { "slug": "getting-started", "label": "Getting Started", "icon": "fa-rocket", "subitems": [
            { "slug": "getting-started", "label": "Quick Start" },
            { "slug": "changelog", "label": "Changelog", "page_title": "Changelog" }
          ] },
          { "slug": "api", "label": "API Reference", "icon": "fa-code", "subitems": [
            { "slug": "api-databases", "label": "Databases" },
            { "slug": "api-collections", "label": "Collections" },
            { "slug": "api-documents", "label": "Documents" },
            { "slug": "api-queries", "label": "Queries" },
            { "slug": "api-indexes", "label": "Indexes" },
            { "slug": "api-vector", "label": "Vector Search" },
            { "slug": "api-scripting", "label": "Scripting" },
            { "slug": "api-blobs", "label": "Blobs" },
            { "slug": "api-transactions", "label": "Transactions" },
            { "slug": "api-cluster", "label": "Cluster" },
            { "slug": "api-auth", "label": "Auth & RBAC" },
            { "slug": "api-triggers", "label": "Triggers" },
            { "slug": "api-monitoring", "label": "Monitoring" }
          ] },
          { "slug": "driver", "label": "Native Driver", "icon": "fa-plug" }
        ] },
        { "title": "Query Languages", "items": [
          { "slug": "sdbql", "label": "SDBQL Reference", "icon": "fa-terminal", "subitems": [
            { "slug": "sdbql-syntax", "label": "Syntax & Basics" },
            { "slug": "sdbql-operators", "label": "Operators" },
            { "slug": "sdbql-mutations", "label": "Data Mutations" },
            { "slug": "sdbql-graphs", "label": "Graph Queries" },
            { "slug": "sdbql-aggregations", "label": "Aggregations" },
            { "slug": "sdbql-cte", "label": "CTEs (WITH)" },
            { "slug": "sdbql-functions-string", "label": "String Functions" },
            { "slug": "sdbql-functions-date", "label": "Date Functions" },
            { "slug": "sdbql-functions-array", "label": "Array Functions" },
            { "slug": "sdbql-functions-numeric", "label": "Numeric Functions" },
            { "slug": "sdbql-functions-geo", "label": "Geo Functions" },
            { "slug": "sdbql-functions-vector", "label": "Vector Functions" },
            { "slug": "sdbql-functions-search", "label": "Search Functions" },
            { "slug": "sdbql-functions-crypto", "label": "Crypto Functions" },
            { "slug": "sdbql-functions-misc", "label": "Other Functions" },
            { "slug": "sdbql-benchmarks", "label": "Benchmarks" }
          ] },
          { "slug": "sql", "label": "SQL Compatibility", "icon": "fa-database" },
          { "slug": "nl-queries", "label": "Natural Language", "icon": "fa-magic" }
        ] },
        { "title": "Core Features", "items": [
          { "slug": "transactions", "label": "ACID & Distributed Tx", "icon": "fa-shield-alt" },
          { "slug": "indexes", "label": "Indexes", "icon": "fa-list-ol" },
          { "slug": "vector-search", "label": "Vector Search", "icon": "fa-brain" },
          { "slug": "hybrid-search", "label": "Hybrid Search", "icon": "fa-layer-group" },
          { "slug": "graphs", "label": "Graphs & Edges", "icon": "fa-project-diagram" },
          { "slug": "graph-rag", "label": "Graph RAG", "icon": "fa-diagram-project" },
          { "slug": "views", "label": "Materialized Views", "icon": "fa-table-cells" },
          { "slug": "scripting", "label": "Lua Scripting", "icon": "fa-scroll", "subitems": [
            { "slug": "scripting-management", "label": "Management API" },
            { "slug": "scripting-services", "label": "Services" },
            { "slug": "scripting-core", "label": "Core API" },
            { "slug": "scripting-database", "label": "Database Access" },
            { "slug": "scripting-ws", "label": "WebSockets" },
            { "slug": "scripting-validation", "label": "Validation" },
            { "slug": "scripting-files", "label": "Files & Media" },
            { "slug": "scripting-utils", "label": "Utilities" },
            { "slug": "scripting-streams", "label": "Streams" },
            { "slug": "scripting-development", "label": "Development" },
            { "slug": "scripting-tutorial-crud", "label": "Tutorial: CRUD API" },
            { "slug": "scripting-tutorial-testing", "label": "Tutorial: Testing" }
          ] },
          { "slug": "triggers", "label": "Triggers", "icon": "fa-bolt" },
          { "slug": "streams", "label": "Stream Processing", "icon": "fa-stream" }
        ] },
        { "title": "Real-Time", "items": [
          { "slug": "changefeeds", "label": "Changefeeds", "icon": "fa-bolt" },
          { "slug": "live-queries", "label": "Live Queries", "icon": "fa-satellite-dish" },
          { "slug": "offline-sync", "label": "Offline Sync", "icon": "fa-sync-alt" }
        ] },
        { "title": "Storage Engines", "items": [
          { "slug": "documents", "label": "Documents Storage", "icon": "fa-file-alt" },
          { "slug": "timeseries", "label": "Time Series", "icon": "fa-chart-line" },
          { "slug": "columnar", "label": "Columnar Storage", "icon": "fa-chart-bar" },
          { "slug": "blobs", "label": "Blob Storage", "icon": "fa-database" }
        ] },
        { "title": "Distributed", "items": [
          { "slug": "cluster", "label": "Clustering", "icon": "fa-network-wired" },
          { "slug": "sharding", "label": "Sharding", "icon": "fa-cubes" }
        ] },
        { "title": "Reference", "items": [
          { "slug": "architecture", "label": "Architecture", "icon": "fa-layer-group" },
          { "slug": "observability", "label": "Observability", "icon": "fa-signal" },
          { "slug": "security", "label": "Security", "icon": "fa-shield-alt" },
          { "slug": "tooling", "label": "CLI Tools", "icon": "fa-wrench" },
          { "slug": "clients", "label": "Official Clients", "icon": "fa-laptop-code", "subitems": [
            { "slug": "clients-laravel", "label": "Laravel Eloquent" },
            { "slug": "clients-go", "label": "Go" },
            { "slug": "clients-python", "label": "Python" },
            { "slug": "clients-nodejs", "label": "Node.js / Bun" },
            { "slug": "clients-php", "label": "PHP" },
            { "slug": "clients-ruby", "label": "Ruby" },
            { "slug": "clients-elixir", "label": "Elixir" },
            { "slug": "clients-rust", "label": "Rust" },
            { "slug": "clients-ios", "label": "iOS (Swift)" },
            { "slug": "clients-android", "label": "Android (Kotlin)" },
            { "slug": "clients-react-native", "label": "React Native" },
            { "slug": "clients-flutter", "label": "Flutter" }
          ] },
          { "slug": "comparison", "label": "DB Comparison", "icon": "fa-scale-balanced" }
        ] }
    ]
end

# Fallback when a page has no usable lead paragraph.
def docs_default_description() -> String
    return "The SoliDB documentation — SDBQL, the HTTP API, Lua scripting, graphs, " +
        "vectors, time-series and clustering, with runnable examples in every official client."
end

# {"title", "description"} for a docs page. `fallback_title` is the controller's
# title, used for pages the sidebar does not list (the docs home).
def docs_meta(slug, fallback_title)
    # Not `title` / `name` / `description`: with locals of those names, every
    # docs page 500'd under `soli test` (2.6.1) though `soli serve` was fine —
    # a bare assignment to a name the runtime already binds replaces it.
    page_name = docs_page_name(slug)
    page_title = page_name.nil? ? fallback_title : page_name + " — SoliDB Docs"
    page_title = "SoliDB Documentation" if slug == "index"
    summary = docs_page_summary(slug) ?? docs_default_description()
    return {"title": page_title, "description": summary}
end

# The sidebar label, qualified by its parent for subpages, whose labels are
# ambiguous alone ("Databases", "Go", "Date Functions"):
# "SDBQL Reference: Date Functions". A label that already has a colon joins
# with a space instead ("Lua Scripting Tutorial: CRUD API"), and an entry's
# "page_title" replaces all of this. nil for a page the sidebar doesn't list.
def docs_page_name(slug)
    slug = "changelog" if slug == "getting-started-changelog"
    found = nil
    docs_nav().each do |section|
        section["items"].each do |item|
            found = item["page_title"] ?? item["label"] if item["slug"] == slug
            found = docs_subpage_name(item, slug) ?? found
        end
    end
    return found
end

def docs_subpage_name(item, slug)
    found = nil
    subitems = item["subitems"] ?? []
    subitems.each do |sub|
        next if sub["slug"] != slug or sub["slug"] == item["slug"]

        joiner = sub["label"].contains(":") ? " " : ": "
        found = sub["page_title"] ?? item["label"] + joiner + sub["label"]
    end
    return found
end

# The page's description, derived from its view source as plain text cut to
# ~160 characters (what search results and share cards show), in order:
#
# 1. its lead: the paragraphs between the <h1> and the first <h2>;
# 2. for an API page, which has no lead but a card per endpoint: the first
#    sentence of each endpoint's summary;
# 3. for a page that opens on an <h2> instead of an <h1>: that section's lead;
# 4. otherwise its section headings.
#
# nil when none of these gives anything; the caller then uses the site-wide
# description.
def docs_page_summary(slug)
    view = slug == "getting-started-changelog" ? "changelog" : slug
    source = File.read("app/views/docs/" + view + ".html.slv") rescue nil
    return nil if source.nil?

    after_h1 = source.split("</h1>")
    has_h1 = after_h1.length > 1
    lead = has_h1 ? docs_lead(after_h1[1].split("<h2")[0]) : nil
    return docs_clip(lead, 160) unless lead.nil?

    endpoints = docs_endpoint_summary(slug, source)
    return docs_clip(endpoints, 160) unless endpoints.nil?

    after_h2 = source.split("</h2>")
    lead = (!has_h1 and after_h2.length > 1) ? docs_lead(after_h2[1].split("<h2")[0]) : nil
    return docs_clip(lead, 160) unless lead.nil?

    sections = docs_section_summary(slug, source)
    return sections.nil? ? nil : docs_clip(sections, 160)
end

# The first paragraphs of `html` as one line of text, or nil. Shorter than 40
# characters is a caption or a label, not a lead; one shorter than 70 is
# completed by the next paragraph (past that, the next one is usually already
# body text or an endpoint card).
def docs_lead(html)
    lead = ""
    for para in Regex.find_all("<p[\\s>][\\s\\S]*?</p>", html)
        text = docs_plain_text(para["match"])
        next if text.chars().length < 40

        lead = lead.blank? ? text : lead + " " + text
        break if lead.chars().length >= 70
    end
    return lead.blank? ? nil : lead
end

# "Databases — 3 SoliDB HTTP API endpoints. Create a new database. List all
# existing databases. …", from each endpoint card's method badge and the
# first sentence of the paragraph after it.
def docs_endpoint_summary(slug, source)
    cards = Regex.find_all(">(GET|POST|PUT|PATCH|DELETE|HEAD)</span>\\s*<code[^>]*>[^<]+</code>[\\s\\S]*?</p>", source)
    return nil if cards.length == 0

    sentences = []
    for card in cards
        para = Regex.find("<p[\\s>][\\s\\S]*?</p>", card["match"])
        next if para.nil?

        sentence = Regex.replace("\\.$", docs_plain_text(para["match"]).split(". ")[0], "")
        next if sentence.blank? or sentences.includes?(sentence)

        sentences.push(sentence + ".")
    end
    return nil if sentences.length == 0

    label = (docs_page_name(slug) ?? "SoliDB").split(": ").last
    count = cards.length == 1 ? "1 SoliDB HTTP API endpoint" : str(cards.length) + " SoliDB HTTP API endpoints"
    return label + " — " + count + ". " + sentences.join(" ")
end

# "Go Client — Getting Started, Core Operations, …": the <h1> and the <h2>
# section titles, for a page that opens without a lead paragraph.
def docs_section_summary(slug, source)
    titles = []
    for heading in Regex.find_all("<h2[\\s>][\\s\\S]*?</h2>", source)
        text = docs_plain_text(heading["match"])
        next if text.blank? or text == "Table of Contents" or titles.includes?(text)

        titles.push(text)
    end
    return nil if titles.length < 2

    h1 = Regex.find("<h1[\\s>][\\s\\S]*?</h1>", source)
    heading = h1.nil? ? (docs_page_name(slug) ?? "SoliDB") : docs_plain_text(h1["match"])
    return heading + " — " + titles.join(", ") + "."
end

# Markup to one line of text: ERB tags dropped, HTML stripped, entities
# decoded (html_unescape leaves named ones like &mdash; alone), whitespace
# collapsed.
def docs_plain_text(html)
    text = Regex.replace_all("<%[\\s\\S]*?%>", html, "")
    text = html_unescape(strip_html(text))
    text = text.gsub("&mdash;", "—").gsub("&ndash;", "–").gsub("&rarr;", "→")
    text = text.gsub("&larr;", "←").gsub("&hellip;", "…").gsub("&nbsp;", " ")
    return Regex.replace_all("\\s+", text, " ").trim()
end

# Cut on a word boundary to at most `limit` characters, ellipsis included.
# Counts characters, not bytes: String#length would cut accented text short.
def docs_clip(text, limit)
    return text if text.chars().length <= limit

    clipped = ""
    for word in text.split(" ")
        candidate = clipped.blank? ? word : clipped + " " + word
        break if candidate.chars().length > limit - 1

        clipped = candidate
    end
    return Regex.replace("[,;:.\\s]+$", clipped, "") + "…"
end
