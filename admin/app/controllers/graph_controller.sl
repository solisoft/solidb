# Graph explorer - interactive traversal of edge collections. The page hosts
# a vis-network canvas; /traverse answers JSON {nodes, edges} fragments that
# the client merges into the canvas as the user explores (double-click a
# vertex to expand its neighborhood).

class GraphController < Controller
  static {
    this.layout = "application"
  }

  # GET /databases/:db/graph
  def show
    this._ctx()
    @title = "Graph · " + @db
    @flash_error = ""
    @flash_notice = ""
    @solidb_down = false
  end

  # POST /databases/:db/graph/traverse
  # Params: start ("collection/key"), edge (edge collection name),
  # direction (outbound/inbound/any), depth (1..5), limit (1..500).
  # Returns JSON: { ok, nodes: [{id,key,collection,doc}], edges: [{id,from,to,collection,doc}] }
  def traverse
    this._ctx()
    edge_collection = (params["edge"] ?? "").trim()
    # The collection name is spliced into the query text (only the start
    # vertex can be a bind var), so it MUST be validated against the real
    # edge-collection list - never trusted from the request.
    if !@edge_collections.includes?(edge_collection)
      return render_json({ "ok": false, "error": "unknown edge collection" })
    end
    direction = this._direction()
    depth = this._clamp((params["depth"] ?? "1").to_int(), 1, 5)
    limit = this._clamp((params["limit"] ?? "100").to_int(), 1, 500)

    # No start vertex? Sample the edge collection instead so the user gets an
    # instant overview to explore from.
    start = (params["start"] ?? "").trim()
    return this._overview(edge_collection, limit) if start.blank?
    if !start.includes?("/")
      return render_json({ "ok": false, "error": "start vertex must look like collection/key" })
    end

    root = this._fetch_vertex(start)
    if root.nil?
      return render_json({ "ok": false, "error": "start vertex " + start + " not found" })
    end

    query = "FOR v, e IN 1.." + str(depth) + " " + direction + " @start " + edge_collection +
            " LIMIT " + str(limit) + " RETURN { \"vertex\": v, \"edge\": e }"
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db),
                                   { "query": query, "bindVars": { "start": start }, "cache": false })
    if !result["ok"]
      return render_json({ "ok": false, "error": result["error"] ?? "request failed" })
    end
    rows = (result["data"] ?? {})["result"] ?? []
    return render_json(this._graph_payload(start, root, rows))
  end

  # POST /databases/:db/graph/demo - seed a small Star Wars graph for demos,
  # then land back on the explorer with the demo preloaded (the query params
  # are picked up by the client and auto-explored). Re-seeding is harmless:
  # every document carries a deterministic _key, so duplicates just 409.
  def seed_demo
    this._ctx()
    SolidbClient.post_api(SolidbEndpoints.collections(@db), { "name": "sw_characters" })
    SolidbClient.post_api(SolidbEndpoints.collections(@db), { "name": "sw_relations", "type": "edge" })
    for character in this._demo_characters()
      SolidbClient.post_api(SolidbEndpoints.documents(@db, "sw_characters"), character)
    end
    for relation in this._demo_relations()
      SolidbClient.post_api(SolidbEndpoints.documents(@db, "sw_relations"), relation)
    end
    return redirect(db_graph_path(@db) + "?edge=sw_relations&start=sw_characters/luke&direction=any&depth=2")
  end

  def _demo_characters
    return [
      { "_key": "luke", "name": "Luke Skywalker", "affiliation": "rebellion", "role": "jedi" },
      { "_key": "leia", "name": "Leia Organa", "affiliation": "rebellion", "role": "general" },
      { "_key": "han", "name": "Han Solo", "affiliation": "rebellion", "role": "smuggler" },
      { "_key": "chewbacca", "name": "Chewbacca", "affiliation": "rebellion", "role": "copilot" },
      { "_key": "vader", "name": "Darth Vader", "affiliation": "empire", "role": "sith" },
      { "_key": "palpatine", "name": "Emperor Palpatine", "affiliation": "empire", "role": "sith lord" },
      { "_key": "obiwan", "name": "Obi-Wan Kenobi", "affiliation": "jedi order", "role": "jedi master" },
      { "_key": "yoda", "name": "Yoda", "affiliation": "jedi order", "role": "grand master" },
      { "_key": "r2d2", "name": "R2-D2", "affiliation": "rebellion", "role": "astromech droid" },
      { "_key": "c3po", "name": "C-3PO", "affiliation": "rebellion", "role": "protocol droid" },
      { "_key": "lando", "name": "Lando Calrissian", "affiliation": "rebellion", "role": "administrator" },
      { "_key": "boba", "name": "Boba Fett", "affiliation": "bounty hunters", "role": "bounty hunter" }
    ]
  end

  # Edge docs as [key, from, to, relation] tuples - expanded below so each
  # entry stays on one readable line.
  def _demo_relations
    tuples = [
      ["vader-father-luke", "vader", "luke", "father_of"],
      ["vader-father-leia", "vader", "leia", "father_of"],
      ["luke-sibling-leia", "luke", "leia", "sibling_of"],
      ["han-loves-leia", "han", "leia", "loves"],
      ["chewie-copilot-han", "chewbacca", "han", "copilot_of"],
      ["obiwan-mentor-luke", "obiwan", "luke", "mentor_of"],
      ["obiwan-mentor-vader", "obiwan", "vader", "mentor_of"],
      ["yoda-mentor-luke", "yoda", "luke", "mentor_of"],
      ["yoda-mentor-obiwan", "yoda", "obiwan", "mentor_of"],
      ["palpatine-master-vader", "palpatine", "vader", "master_of"],
      ["luke-owns-r2d2", "luke", "r2d2", "owns"],
      ["r2d2-friend-c3po", "r2d2", "c3po", "friend_of"],
      ["lando-friend-han", "lando", "han", "friend_of"],
      ["boba-hunts-han", "boba", "han", "hunts"]
    ]
    relations = []
    for tuple in tuples
      relations.push({ "_key": tuple[0], "_from": "sw_characters/" + tuple[1],
                       "_to": "sw_characters/" + tuple[2], "relation": tuple[3] })
    end
    return relations
  end

  # Direction keyword for the query, whitelisted (defaults to ANY).
  def _direction
    keywords = { "outbound": "OUTBOUND", "inbound": "INBOUND", "any": "ANY" }
    return keywords[(params["direction"] ?? "any").trim().downcase()] ?? "ANY"
  end

  def _clamp(value, min_value, max_value)
    return min_value if value < min_value
    return max_value if value > max_value
    return value
  end

  # The traversal never returns the start vertex itself - fetch it directly
  # so the canvas always has a root. nil when the id does not resolve.
  def _fetch_vertex(vertex_id)
    parts = vertex_id.split("/")
    result = SolidbClient.get_api(SolidbEndpoints.document(@db, parts[0], parts[1]))
    return nil unless result["ok"]
    return result["data"]
  end

  # Overview mode: the first <limit> edges of the collection, with both
  # endpoint documents resolved in-query.
  def _overview(edge_collection, limit)
    query = "FOR e IN " + edge_collection + " LIMIT " + str(limit) +
            " RETURN { \"edge\": e, \"from\": DOCUMENT(e._from), \"to\": DOCUMENT(e._to) }"
    result = SolidbClient.post_api(SolidbEndpoints.cursor(@db), { "query": query, "cache": false })
    if !result["ok"]
      return render_json({ "ok": false, "error": result["error"] ?? "request failed" })
    end
    rows = (result["data"] ?? {})["result"] ?? []
    vertex_docs = {}
    edge_docs = {}
    root_id = ""
    for row in rows
      edge = row["edge"] ?? {}
      edge_id = this._plain_id(edge["_id"] ?? "")
      edge_docs[edge_id] = edge unless edge_id.blank?
      # The first edge's tail becomes the picked start vertex: the client
      # writes it back into the toolbar so the next Explore is anchored.
      root_id = edge["_from"] ?? "" if root_id.blank?
      for vertex in [row["from"] ?? {}, row["to"] ?? {}]
        vertex_id = this._plain_id(vertex["_id"] ?? "")
        vertex_docs[vertex_id] = vertex unless vertex_id.blank?
      end
    end
    return render_json(this._assemble(root_id, vertex_docs, edge_docs))
  end

  # Dedupe vertices/edges by _id and shape them for the client. Edge endpoints
  # that the traversal did not visit (depth cut) become placeholder nodes so
  # every edge can render.
  def _graph_payload(start, root, rows)
    vertex_docs = {}
    vertex_docs[start] = root
    edge_docs = {}
    for row in rows
      vertex = row["vertex"] ?? {}
      edge = row["edge"] ?? {}
      # The cursor prefixes _id with the database name ("db:people/bob");
      # _from/_to are plain ("people/bob"). Normalize so they join up.
      vertex_id = this._plain_id(vertex["_id"] ?? "")
      edge_id = this._plain_id(edge["_id"] ?? "")
      vertex_docs[vertex_id] = vertex unless vertex_id.blank?
      edge_docs[edge_id] = edge unless edge_id.blank?
    end
    return this._assemble(start, vertex_docs, edge_docs)
  end

  def _assemble(root_id, vertex_docs, edge_docs)
    nodes = []
    for vertex_id in vertex_docs.keys()
      nodes.push(this._node_entry(vertex_id, vertex_docs[vertex_id]))
    end
    edges = []
    for edge_id in edge_docs.keys()
      edge = edge_docs[edge_id]
      # Endpoints the traversal did not visit (depth cut) become placeholder
      # nodes so every edge can render; the {} marker prevents duplicates.
      for endpoint in [edge["_from"] ?? "", edge["_to"] ?? ""]
        if !endpoint.blank? && vertex_docs[endpoint].nil?
          vertex_docs[endpoint] = {}
          nodes.push(this._node_entry(endpoint, nil))
        end
      end
      edges.push({ "id": edge_id, "from": edge["_from"] ?? "", "to": edge["_to"] ?? "",
                   "collection": edge_id.split("/")[0], "doc": edge })
    end
    return { "ok": true, "root": root_id, "nodes": nodes, "edges": edges }
  end

  def _node_entry(vertex_id, doc)
    parts = vertex_id.split("/")
    return { "id": vertex_id, "key": parts[1] ?? vertex_id, "collection": parts[0], "doc": doc }
  end

  # "db:people/bob" -> "people/bob"; plain ids pass through.
  def _plain_id(full_id)
    parts = full_id.split(":")
    return parts[parts.length() - 1]
  end

  # Route context + collection lists for the toolbar: edge collections are
  # traversable, document collections feed the start-vertex suggestions.
  def _ctx
    @db = params["db"] ?? ""
    @databases = AdminContext.database_names()
    result = SolidbClient.get_api(SolidbEndpoints.collections(@db))
    all_collections = (result["data"] ?? {})["collections"] ?? []
    @edge_collections = []
    @vertex_collections = []
    for coll in all_collections
      coll_name = coll["name"] ?? ""
      if !coll_name.starts_with("_")
        coll_type = coll["type"] ?? "document"
        @edge_collections.push(coll_name) if coll_type == "edge"
        @vertex_collections.push(coll_name) if coll_type == "document"
      end
    end
  end
end
