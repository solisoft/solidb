# Graph explorer against a scratch database seeded with a small social graph:
#   alice -knows-> bob -knows-> carol, dave -knows-> alice
describe("GraphController") do
  before_all() do
    SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_graph" })
    SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_graph"), { "name": "people" })
    SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_graph"), { "name": "knows", "type": "edge" })
    for person in ["alice", "bob", "carol", "dave"]
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_graph", "people"),
                            { "_key": person, "name": person })
    end
    SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_graph", "knows"),
                          { "_from": "people/alice", "_to": "people/bob", "since": 2020 })
    SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_graph", "knows"),
                          { "_from": "people/bob", "_to": "people/carol", "since": 2021 })
    SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_graph", "knows"),
                          { "_from": "people/dave", "_to": "people/alice", "since": 2022 })
  end

  after_all() do
    SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_graph"))
  end

  before_each() do
    as_guest()
  end

  describe("GET /databases/:db/graph") do
    test("renders the explorer with edge collections and vertex suggestions") do
      response = get("/databases/admin_spec_graph/graph")
      assert_eq(res_status(response), 200)
      body = res_body(response)
      assert_contains(body, "Graph explorer")
      assert_contains(body, "knows")
      assert_contains(body, "people/")
      assert_contains(body, "graph-canvas")
    end

    test("shows an empty state when the db has no edge collections") do
      SolidbClient.post_api(SolidbEndpoints.database_create(), { "name": "admin_spec_graph_bare" })
      response = get("/databases/admin_spec_graph_bare/graph")
      assert_eq(res_status(response), 200)
      assert_contains(res_body(response), "no edge collections")
      SolidbClient.delete_api(SolidbEndpoints.database("admin_spec_graph_bare"))
    end
  end

  describe("POST /databases/:db/graph/traverse") do
    test("outbound traversal returns the reachable subgraph") do
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "people/alice", "edge": "knows", "direction": "outbound", "depth": "2" })
      assert_eq(res_status(response), 200)
      data = res_json(response)
      assert(data["ok"])
      assert_eq(data["root"], "people/alice")
      node_ids = data["nodes"].map do |node| node["id"] end
      assert_contains(node_ids, "people/alice")
      assert_contains(node_ids, "people/bob")
      assert_contains(node_ids, "people/carol")
      assert_not(node_ids.includes?("people/dave"))
      assert_eq(data["edges"].length(), 2)
      assert_eq(data["edges"][0]["collection"], "knows")
    end

    test("inbound traversal walks edges backwards") do
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "people/alice", "edge": "knows", "direction": "inbound", "depth": "1" })
      data = res_json(response)
      assert(data["ok"])
      node_ids = data["nodes"].map do |node| node["id"] end
      assert_contains(node_ids, "people/dave")
      assert_not(node_ids.includes?("people/bob"))
    end

    test("unknown direction falls back to any") do
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "people/alice", "edge": "knows", "direction": "sideways", "depth": "1" })
      data = res_json(response)
      assert(data["ok"])
      node_ids = data["nodes"].map do |node| node["id"] end
      assert_contains(node_ids, "people/bob")
      assert_contains(node_ids, "people/dave")
    end

    test("depth and limit are clamped instead of erroring") do
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "people/alice", "edge": "knows", "direction": "outbound",
                        "depth": "99", "limit": "99999" })
      data = res_json(response)
      assert(data["ok"])
      assert_eq(data["edges"].length(), 2)
    end

    test("rejects an edge collection that does not exist") do
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "people/alice", "edge": "people; REMOVE", "depth": "1" })
      data = res_json(response)
      assert_not(data["ok"])
      assert_contains(data["error"], "unknown edge collection")
    end

    test("blank start vertex samples the edge collection and picks a root") do
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "", "edge": "knows", "depth": "1" })
      data = res_json(response)
      assert(data["ok"])
      assert_eq(data["edges"].length(), 3)
      node_ids = data["nodes"].map do |node| node["id"] end
      for person in ["people/alice", "people/bob", "people/carol", "people/dave"]
        assert_contains(node_ids, person)
      end
      # A start vertex is picked for the client to anchor the next explore.
      assert_contains(node_ids, data["root"])
    end

    test("rejects a start vertex without a collection prefix") do
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "alice", "edge": "knows", "depth": "1" })
      data = res_json(response)
      assert_not(data["ok"])
      assert_contains(data["error"], "collection/key")
    end

    test("surfaces a missing start vertex") do
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "people/nobody", "edge": "knows", "depth": "1" })
      data = res_json(response)
      assert_not(data["ok"])
      assert_contains(data["error"], "not found")
    end

    test("joins nodes whose keys contain colons (code-graph ids)") do
      # Regression: _plain_id used to split on every ":" and keep the last
      # segment, so a cursor id "db:code_nodes/external:Some.Ns" normalized to
      # "Some.Ns" - a slashless id that crashed _node_entry (index out of
      # bounds) and never matched an edge's _from/_to, orphaning every node.
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_graph"), { "name": "code_nodes" })
      SolidbClient.post_api(SolidbEndpoints.collections("admin_spec_graph"), { "name": "code_edges", "type": "edge" })
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_graph", "code_nodes"),
                            { "_key": "file:api:Foo.cs", "name": "Foo.cs" })
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_graph", "code_nodes"),
                            { "_key": "external:Some.Namespace", "name": "Some.Namespace" })
      SolidbClient.post_api(SolidbEndpoints.documents("admin_spec_graph", "code_edges"),
                            { "_from": "code_nodes/file:api:Foo.cs", "_to": "code_nodes/external:Some.Namespace" })

      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "code_nodes/file:api:Foo.cs", "edge": "code_edges",
                        "direction": "outbound", "depth": "1" })
      assert_eq(res_status(response), 200)
      data = res_json(response)
      assert(data["ok"])
      node_ids = data["nodes"].map do |node| node["id"] end
      # Full "collection/key" ids survive normalization and line up with the edge.
      assert_contains(node_ids, "code_nodes/file:api:Foo.cs")
      assert_contains(node_ids, "code_nodes/external:Some.Namespace")
      # The key's last colon segment must NOT leak out as a bare, slashless id.
      assert_not(node_ids.includes?("Some.Namespace"))
      edge = data["edges"][0]
      assert_eq(edge["from"], "code_nodes/file:api:Foo.cs")
      assert_eq(edge["to"], "code_nodes/external:Some.Namespace")
      # The traversed neighbor resolves to its document, not a dashed placeholder,
      # and its rendered key is everything after the collection prefix.
      neighbor = data["nodes"].filter do |node| node["id"] == "code_nodes/external:Some.Namespace" end
      assert_not_null(neighbor[0]["doc"])
      assert_eq(neighbor[0]["key"], "external:Some.Namespace")
    end
  end

  describe("POST /databases/:db/graph/demo") do
    test("seeds the star wars graph and redirects into it") do
      response = post("/databases/admin_spec_graph/graph/demo", {})
      assert(res_redirect?(response))
      assert_contains(res_location(response), "edge=sw_relations")
      assert_contains(res_location(response), "start=sw_characters/luke")

      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "sw_characters/luke", "edge": "sw_relations",
                        "direction": "any", "depth": "1" })
      data = res_json(response)
      assert(data["ok"])
      node_ids = data["nodes"].map do |node| node["id"] end
      assert_contains(node_ids, "sw_characters/vader")
      assert_contains(node_ids, "sw_characters/leia")
      # luke's depth-1 neighborhood: father, sibling, two mentors, his droid.
      assert_eq(data["edges"].length(), 5)
    end

    test("re-seeding does not duplicate documents") do
      post("/databases/admin_spec_graph/graph/demo", {})
      post("/databases/admin_spec_graph/graph/demo", {})
      response = post("/databases/admin_spec_graph/graph/traverse",
                      { "start": "sw_characters/luke", "edge": "sw_relations",
                        "direction": "any", "depth": "1" })
      data = res_json(response)
      assert(data["ok"])
      assert_eq(data["edges"].length(), 5)
    end
  end
end
