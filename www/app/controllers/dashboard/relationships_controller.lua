-- Dashboard Relationships Controller
-- Builds an entity-relationship style map of all collections, inferring
-- relations between them from index keys (foreign-key style fields) and from
-- edge collection _from/_to references.
local DashboardBaseController = require("dashboard.base_controller")
local RelationshipsController = DashboardBaseController:extend()

-- ---------------------------------------------------------------------------
-- Naming helpers used to match an indexed field back to a collection name.
-- ---------------------------------------------------------------------------

local function depluralize(s)
  if s:match("ies$") then
    return (s:gsub("ies$", "y"))
  elseif s:match("ses$") or s:match("xes$") or s:match("zes$") or s:match("ches$") or s:match("shes$") then
    return (s:gsub("es$", ""))
  elseif s:match("s$") and not s:match("ss$") then
    return (s:gsub("s$", ""))
  end
  return s
end

local function pluralize(s)
  if s:match("y$") then
    return (s:gsub("y$", "ies"))
  elseif s:match("s$") or s:match("x$") or s:match("z$") or s:match("ch$") or s:match("sh$") then
    return s .. "es"
  end
  return s .. "s"
end

-- Strip a foreign-key style suffix off a field name and return the base noun,
-- or nil if the field doesn't look like a reference. Handles snake_case
-- (user_id, account_key) and camelCase (userId, accountRef). Nested paths use
-- the trailing segment (e.g. "owner.user_id" -> "user").
local function fk_base(field)
  if type(field) ~= "string" or field == "" then
    return nil
  end

  -- Use the last path segment for nested fields.
  local seg = field:match("([^.]+)$") or field

  -- Skip document metadata fields (_id/_key/_rev/_from/_to handled elsewhere).
  if seg:sub(1, 1) == "_" then
    return nil
  end

  local lower = seg:lower()

  -- snake_case suffixes
  local snake_suffixes = { "_ids", "_id", "_keys", "_key", "_refs", "_ref", "_fk", "_uuid", "_guid" }
  for _, suf in ipairs(snake_suffixes) do
    if #lower > #suf and lower:sub(-#suf) == suf then
      return lower:sub(1, #lower - #suf)
    end
  end

  -- camelCase / PascalCase suffixes (preserve original case to detect the boundary)
  local camel_suffixes = { "Ids", "Id", "Keys", "Key", "Refs", "Ref", "UUID", "Uuid", "Guid" }
  for _, suf in ipairs(camel_suffixes) do
    if #seg > #suf and seg:sub(-#suf) == suf then
      local base = seg:sub(1, #seg - #suf)
      if base ~= "" then
        return base:lower()
      end
    end
  end

  return nil
end

-- ---------------------------------------------------------------------------
-- Main page
-- ---------------------------------------------------------------------------
function RelationshipsController:index()
  self.layout = "dashboard"
  local db = self:get_db()
  self:render("dashboard/relationships", {
    title = "Relationships - " .. db,
    db = db,
    current_page = "relationships"
  })
end

-- ---------------------------------------------------------------------------
-- JSON data endpoint: nodes (collections) + edges (inferred relations)
-- ---------------------------------------------------------------------------
function RelationshipsController:data()
  local db = self:get_db()
  local include_system = self.params.include_system == "true"

  -- 1. Load all collections ------------------------------------------------
  local collections = {}
  local status, _, body = self:fetch_api("/_api/database/" .. db .. "/collection")
  if status == 200 then
    local ok, parsed = pcall(DecodeJson, body)
    if ok and parsed then
      collections = parsed.collections or parsed or {}
    end
  end

  -- Build a lookup table from normalized name forms -> actual collection name.
  local coll_map = {}
  local nodes = {}
  local node_index = {}      -- name -> position in nodes (for quick lookups)
  local edge_collections = {}

  local function register_name(form, actual)
    if form and form ~= "" and not coll_map[form] then
      coll_map[form] = actual
    end
  end

  for _, c in ipairs(collections) do
    local name = c.name
    if name and (include_system or name:sub(1, 1) ~= "_") then
      local lower = name:lower()
      register_name(lower, name)
      register_name(depluralize(lower), name)
      register_name(pluralize(lower), name)

      local is_edge = c.type == "edge" or c.type == 3
      local node = {
        id = name,
        label = name,
        type = is_edge and "edge" or "document",
        count = c.count or c.document_count or 0,
        indexCount = 0,
        relationCount = 0
      }
      nodes[#nodes + 1] = node
      node_index[name] = node
      if is_edge then
        edge_collections[#edge_collections + 1] = name
      end
    end
  end

  local function resolve(base)
    if not base or base == "" then
      return nil
    end
    return coll_map[base] or coll_map[depluralize(base)] or coll_map[pluralize(base)]
  end

  -- 2. Inspect indexes of each collection to infer FK relations ------------
  local edges = {}
  local seen_edges = {}

  local function add_edge(source, target, label, kind, extra)
    -- Stable key so we don't draw duplicate relations.
    local key = kind .. "|" .. source .. "|" .. target .. "|" .. (label or "")
    if seen_edges[key] then
      return
    end
    seen_edges[key] = true
    local e = {
      id = "e" .. (#edges + 1),
      source = source,
      target = target,
      label = label,
      kind = kind
    }
    if extra then
      for k, v in pairs(extra) do
        e[k] = v
      end
    end
    edges[#edges + 1] = e
    if node_index[source] then
      node_index[source].relationCount = node_index[source].relationCount + 1
    end
  end

  for _, node in ipairs(nodes) do
    local name = node.id
    local idx_status, _, idx_body = self:fetch_api("/_api/database/" .. db .. "/index/" .. name)
    if idx_status == 200 then
      local ok, parsed = pcall(DecodeJson, idx_body)
      if ok and parsed then
        local indexes = parsed.indexes or parsed or {}
        for _, idx in ipairs(indexes) do
          node.indexCount = node.indexCount + 1

          -- Collect the fields covered by this index.
          local fields = {}
          if type(idx.fields) == "table" then
            for _, f in ipairs(idx.fields) do
              fields[#fields + 1] = f
            end
          end
          if idx.field and idx.field ~= "" then
            fields[#fields + 1] = idx.field
          end

          local idx_type = idx.index_type or idx.type or "hash"
          if type(idx_type) ~= "string" then
            idx_type = "hash"
          end

          for _, field in ipairs(fields) do
            local base = fk_base(field)
            local target = resolve(base)
            -- Don't draw self-loops from a collection's own-name suffix
            -- (e.g. "users" collection with a "user_id" pk-ish field) unless
            -- the field truly points elsewhere; self references are kept.
            if target and target ~= "" then
              add_edge(name, target, field, "fk", {
                index = idx.name,
                unique = idx.unique == true,
                indexType = idx_type
              })
            end
          end
        end
      end
    end
  end

  -- 3. Edge collections: sample _from/_to to connect vertex collections ----
  for _, edge_coll in ipairs(edge_collections) do
    local query = string.format(
      "FOR d IN %s FILTER d._from != null && d._to != null LIMIT 50 RETURN { f: d._from, t: d._to }",
      edge_coll
    )
    local q_status, _, q_body = self:fetch_api("/_api/database/" .. db .. "/cursor", {
      method = "POST",
      body = EncodeJson({ query = query })
    })
    if q_status == 200 then
      local ok, parsed = pcall(DecodeJson, q_body)
      if ok and parsed and parsed.result then
        local from_seen = {}
        local to_seen = {}
        for _, row in ipairs(parsed.result) do
          local from_coll = type(row.f) == "string" and row.f:match("^([^/]+)/") or nil
          local to_coll = type(row.t) == "string" and row.t:match("^([^/]+)/") or nil
          if from_coll and node_index[from_coll] and not from_seen[from_coll] then
            from_seen[from_coll] = true
            add_edge(from_coll, edge_coll, "_from", "graph")
          end
          if to_coll and node_index[to_coll] and not to_seen[to_coll] then
            to_seen[to_coll] = true
            add_edge(edge_coll, to_coll, "_to", "graph")
          end
        end
      end
    end
  end

  -- 4. Summary stats -------------------------------------------------------
  local fk_count = 0
  local graph_count = 0
  for _, e in ipairs(edges) do
    if e.kind == "graph" then
      graph_count = graph_count + 1
    else
      fk_count = fk_count + 1
    end
  end

  -- Building this graph costs one API round-trip per collection (index
  -- listing) plus one per edge collection (sample query). Redbean forks per
  -- connection, so an in-process Lua memo would not survive across requests;
  -- instead let the browser cache the response briefly so re-renders and
  -- quick navigations don't redo the N+1 walk.
  SetHeader("Cache-Control", "private, max-age=30")

  self:json({
    nodes = nodes,
    edges = edges,
    stats = {
      collections = #nodes,
      edgeCollections = #edge_collections,
      relations = #edges,
      fkRelations = fk_count,
      graphRelations = graph_count
    }
  })
end

return RelationshipsController
