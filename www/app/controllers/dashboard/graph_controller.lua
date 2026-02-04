-- Dashboard Graph Controller
-- Handles graph visualization, traversal, and vertex/edge CRUD
local DashboardBaseController = require("dashboard.base_controller")
local GraphController = DashboardBaseController:extend()

-- Color palette for different edge collections
local EDGE_COLORS = {
  "#6366f1", -- indigo
  "#10b981", -- emerald
  "#f59e0b", -- amber
  "#ef4444", -- red
  "#8b5cf6", -- violet
  "#ec4899", -- pink
  "#06b6d4", -- cyan
  "#84cc16"  -- lime
}

-- Graph Explorer main page
function GraphController:index()
  self.layout = "dashboard"
  local db = self:get_db()

  -- Fetch collections to populate dropdowns
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection")
  local vertex_collections = {}
  local edge_collections = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      local collections = data.collections or data or {}
      for _, coll in ipairs(collections) do
        -- Filter out system collections
        if coll.name and coll.name:sub(1, 1) ~= "_" then
          if coll.type == "edge" then
            table.insert(edge_collections, coll.name)
          else
            table.insert(vertex_collections, coll.name)
          end
        end
      end
    end
  end

  self:render("dashboard/graph", {
    title = "Graph Explorer - " .. db,
    db = db,
    current_page = "graph",
    vertex_collections = vertex_collections,
    edge_collections = edge_collections
  })
end

-- Get collections list (JSON API for dynamic loading)
function GraphController:collections()
  local db = self:get_db()
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection")

  local vertex_collections = {}
  local edge_collections = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      local collections = data.collections or data or {}
      for _, coll in ipairs(collections) do
        if coll.name and coll.name:sub(1, 1) ~= "_" then
          if coll.type == "edge" then
            table.insert(edge_collections, coll.name)
          else
            table.insert(vertex_collections, coll.name)
          end
        end
      end
    end
  end

  self:json({
    vertex_collections = vertex_collections,
    edge_collections = edge_collections
  })
end

-- Fetch graph data (vertices + edges from multiple collections)
function GraphController:data()
  local db = self:get_db()
  local vertex_coll = self.params.vertex_collection
  local edge_colls_raw = self.params.edge_collections or ""
  local start_vertex = self.params.start_vertex
  local min_depth = tonumber(self.params.min_depth) or 1
  local max_depth = tonumber(self.params.max_depth) or 2
  local direction = self.params.direction or "ANY"
  local node_limit = tonumber(self.params.limit) or 50

  -- Parse edge collections (comma-separated)
  local edge_colls = {}
  if type(edge_colls_raw) == "string" and edge_colls_raw ~= "" then
    for coll in string.gmatch(edge_colls_raw, "[^,]+") do
      local trimmed = coll:match("^%s*(.-)%s*$")
      if trimmed ~= "" then
        table.insert(edge_colls, trimmed)
      end
    end
  elseif type(edge_colls_raw) == "table" then
    edge_colls = edge_colls_raw
  end

  local all_nodes = {}
  local all_edges = {}
  local seen_nodes = {}
  local legend = {}

  -- If we have a start vertex, do graph traversal
  if start_vertex and start_vertex ~= "" and #edge_colls > 0 then
    for idx, edge_coll in ipairs(edge_colls) do
      local color = EDGE_COLORS[((idx - 1) % #EDGE_COLORS) + 1]
      table.insert(legend, { collection = edge_coll, color = color })

      -- Build traversal query
      local query = string.format([[
        FOR v, e IN %d..%d %s '%s' %s
        OPTIONS { uniqueVertices: "global" }
        RETURN { vertex: v, edge: e }
      ]], min_depth, max_depth, direction, start_vertex, edge_coll)

      local status, _, body = self:fetch_api("/_api/database/" .. db .. "/cursor", {
        method = "POST",
        body = EncodeJson({ query = query })
      })

      if status == 200 then
        local ok, data = pcall(DecodeJson, body)
        if ok and data and data.result then
          for _, item in ipairs(data.result) do
            -- Add vertex if not seen
            if item.vertex and item.vertex._id and not seen_nodes[item.vertex._id] then
              seen_nodes[item.vertex._id] = true
              local label = item.vertex.name or item.vertex.title or item.vertex._key
              table.insert(all_nodes, {
                id = item.vertex._id,
                label = tostring(label),
                collection = item.vertex._id:match("^([^/]+)/"),
                data = item.vertex
              })
            end

            -- Add edge
            if item.edge and item.edge._from and item.edge._to then
              table.insert(all_edges, {
                id = item.edge._id or (item.edge._from .. "->" .. item.edge._to),
                source = item.edge._from,
                target = item.edge._to,
                collection = edge_coll,
                color = color,
                data = item.edge
              })
            end
          end
        end
      end
    end

    -- Also add the start vertex if not already included
    if not seen_nodes[start_vertex] then
      local coll_name, key = start_vertex:match("^([^/]+)/(.+)$")
      if coll_name and key then
        local v_status, _, v_body = self:fetch_api("/_api/database/" .. db .. "/document/" .. coll_name .. "/" .. key)
        if v_status == 200 then
          local ok, vertex = pcall(DecodeJson, v_body)
          if ok and vertex then
            seen_nodes[start_vertex] = true
            local label = vertex.name or vertex.title or vertex._key
            table.insert(all_nodes, 1, {
              id = vertex._id,
              label = tostring(label),
              collection = coll_name,
              data = vertex,
              isStart = true
            })
          end
        end
      end
    end

  -- No start vertex - load sample from vertex collection
  elseif vertex_coll and vertex_coll ~= "" then
    -- Fetch vertices
    local v_query = string.format("FOR doc IN %s LIMIT %d RETURN doc", vertex_coll, node_limit)
    local v_status, _, v_body = self:fetch_api("/_api/database/" .. db .. "/cursor", {
      method = "POST",
      body = EncodeJson({ query = v_query })
    })

    if v_status == 200 then
      local ok, v_data = pcall(DecodeJson, v_body)
      if ok and v_data and v_data.result then
        for _, vertex in ipairs(v_data.result) do
          if vertex._id then
            seen_nodes[vertex._id] = true
            local label = vertex.name or vertex.title or vertex._key
            table.insert(all_nodes, {
              id = vertex._id,
              label = tostring(label),
              collection = vertex_coll,
              data = vertex
            })
          end
        end
      end
    end

    -- Fetch edges between loaded vertices
    for idx, edge_coll in ipairs(edge_colls) do
      local color = EDGE_COLORS[((idx - 1) % #EDGE_COLORS) + 1]
      table.insert(legend, { collection = edge_coll, color = color })

      local e_query = string.format("FOR e IN %s LIMIT %d RETURN e", edge_coll, node_limit * 2)
      local e_status, _, e_body = self:fetch_api("/_api/database/" .. db .. "/cursor", {
        method = "POST",
        body = EncodeJson({ query = e_query })
      })

      if e_status == 200 then
        local ok, e_data = pcall(DecodeJson, e_body)
        if ok and e_data and e_data.result then
          for _, edge in ipairs(e_data.result) do
            -- Only include edges where both vertices are loaded
            if edge._from and edge._to and seen_nodes[edge._from] and seen_nodes[edge._to] then
              table.insert(all_edges, {
                id = edge._id or (edge._from .. "->" .. edge._to),
                source = edge._from,
                target = edge._to,
                collection = edge_coll,
                color = color,
                data = edge
              })
            end
          end
        end
      end
    end
  end

  self:json({
    nodes = all_nodes,
    edges = all_edges,
    legend = legend
  })
end

-- Execute graph traversal and return path
function GraphController:traverse()
  local db = self:get_db()
  local start_vertex = self.params.start_vertex
  local edge_collection = self.params.edge_collection
  local direction = self.params.direction or "OUTBOUND"
  local min_depth = tonumber(self.params.min_depth) or 1
  local max_depth = tonumber(self.params.max_depth) or 3

  if not start_vertex or start_vertex == "" then
    self:json({ error = "Start vertex is required" })
    return
  end

  if not edge_collection or edge_collection == "" then
    self:json({ error = "Edge collection is required" })
    return
  end

  local query = string.format([[
    FOR v, e, p IN %d..%d %s '%s' %s
    OPTIONS { uniqueVertices: "global" }
    RETURN { vertex: v, edge: e, path: p }
  ]], min_depth, max_depth, direction, start_vertex, edge_collection)

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/cursor", {
    method = "POST",
    body = EncodeJson({ query = query })
  })

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      self:json({
        result = data.result or {},
        count = #(data.result or {})
      })
    else
      self:json({ error = "Failed to parse response" })
    end
  else
    local ok, err_data = pcall(DecodeJson, body or "")
    local error_msg = "Traversal failed"
    if ok and err_data and err_data.error then
      error_msg = err_data.error
    end
    self:json({ error = error_msg })
  end
end

-- Create vertex modal
function GraphController:modal_vertex()
  local db = self:get_db()

  -- Get vertex collections
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection")
  local vertex_collections = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      local collections = data.collections or data or {}
      for _, coll in ipairs(collections) do
        if coll.name and coll.name:sub(1, 1) ~= "_" and coll.type ~= "edge" then
          table.insert(vertex_collections, coll.name)
        end
      end
    end
  end

  self:render_partial("dashboard/_modal_graph_vertex", {
    db = db,
    vertex_collections = vertex_collections,
    preselected_collection = self.params.collection
  })
end

-- Create edge modal
function GraphController:modal_edge()
  local db = self:get_db()

  -- Get edge collections
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection")
  local edge_collections = {}

  if status == 200 then
    local ok, data = pcall(DecodeJson, body)
    if ok and data then
      local collections = data.collections or data or {}
      for _, coll in ipairs(collections) do
        if coll.name and coll.name:sub(1, 1) ~= "_" and coll.type == "edge" then
          table.insert(edge_collections, coll.name)
        end
      end
    end
  end

  self:render_partial("dashboard/_modal_graph_edge", {
    db = db,
    edge_collections = edge_collections,
    preselected_collection = self.params.collection,
    from_vertex = self.params.from,
    to_vertex = self.params.to
  })
end

-- Create new vertex
function GraphController:create_vertex()
  local db = self:get_db()
  local collection = self.params.collection
  local doc_json = self.params.document

  if not collection or collection == "" then
    self:json({ error = "Collection is required" })
    return
  end

  -- Parse document JSON
  local ok, doc = pcall(DecodeJson, doc_json or "{}")
  if not ok then
    self:json({ error = "Invalid JSON document" })
    return
  end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/document/" .. collection, {
    method = "POST",
    body = EncodeJson(doc)
  })

  if status == 200 or status == 201 then
    local ok_res, result = pcall(DecodeJson, body)
    if ok_res and result then
      -- Merge the returned metadata with original doc
      doc._id = result._id
      doc._key = result._key
      doc._rev = result._rev

      self:json({
        success = true,
        vertex = {
          id = result._id,
          label = doc.name or doc.title or result._key,
          collection = collection,
          data = doc
        }
      })
    else
      self:json({ error = "Failed to parse response" })
    end
  else
    local ok_err, err_data = pcall(DecodeJson, body or "")
    local error_msg = "Failed to create vertex"
    if ok_err and err_data and err_data.error then
      error_msg = err_data.error
    end
    self:json({ error = error_msg })
  end
end

-- Create new edge
function GraphController:create_edge()
  local db = self:get_db()
  local collection = self.params.collection
  local from_vertex = self.params._from or self.params.from_vertex
  local to_vertex = self.params._to or self.params.to_vertex
  local doc_json = self.params.document

  if not collection or collection == "" then
    self:json({ error = "Collection is required" })
    return
  end

  if not from_vertex or from_vertex == "" then
    self:json({ error = "From vertex is required" })
    return
  end

  if not to_vertex or to_vertex == "" then
    self:json({ error = "To vertex is required" })
    return
  end

  -- Parse additional document fields
  local doc = {}
  if doc_json and doc_json ~= "" then
    local ok, parsed = pcall(DecodeJson, doc_json)
    if ok then
      doc = parsed
    end
  end

  doc._from = from_vertex
  doc._to = to_vertex

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/document/" .. collection, {
    method = "POST",
    body = EncodeJson(doc)
  })

  if status == 200 or status == 201 then
    local ok_res, result = pcall(DecodeJson, body)
    if ok_res and result then
      doc._id = result._id
      doc._key = result._key
      doc._rev = result._rev

      self:json({
        success = true,
        edge = {
          id = result._id,
          source = from_vertex,
          target = to_vertex,
          collection = collection,
          data = doc
        }
      })
    else
      self:json({ error = "Failed to parse response" })
    end
  else
    local ok_err, err_data = pcall(DecodeJson, body or "")
    local error_msg = "Failed to create edge"
    if ok_err and err_data and err_data.error then
      error_msg = err_data.error
    end
    self:json({ error = error_msg })
  end
end

-- Delete vertex
function GraphController:delete_vertex()
  local db = self:get_db()
  local collection = self.params.collection
  local key = self.params.key

  if not collection or not key then
    self:json({ error = "Collection and key are required" })
    return
  end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/document/" .. collection .. "/" .. key, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    self:json({ success = true })
  else
    local ok_err, err_data = pcall(DecodeJson, body or "")
    local error_msg = "Failed to delete vertex"
    if ok_err and err_data and err_data.error then
      error_msg = err_data.error
    end
    self:json({ error = error_msg })
  end
end

-- Delete edge
function GraphController:delete_edge()
  local db = self:get_db()
  local collection = self.params.collection
  local key = self.params.key

  if not collection or not key then
    self:json({ error = "Collection and key are required" })
    return
  end

  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/document/" .. collection .. "/" .. key, {
    method = "DELETE"
  })

  if status == 200 or status == 204 then
    self:json({ success = true })
  else
    local ok_err, err_data = pcall(DecodeJson, body or "")
    local error_msg = "Failed to delete edge"
    if ok_err and err_data and err_data.error then
      error_msg = err_data.error
    end
    self:json({ error = error_msg })
  end
end

return GraphController
