local Controller = require("controller")
local PagesController = Controller:extend()
local Page = require("models.page")
local AuthHelper = require("helpers.auth_helper")

local function get_current_user()
  return AuthHelper.get_current_user()
end

-- Index: Dashboard / Root pages
function PagesController:index()
  local current_user = get_current_user()
  
  -- Fetch root pages (pages with no parent)
  -- Note: where({parent_id = nil}) might be empty table, fetching all. 
  -- We use explicit SDBQL for safety or filter by checking if parent_id is missing/null.
  -- In this ORM, we can often use query lists. Let's try explicit filter function if possible, 
  -- or just filter in memory if dataset is small (MVP).
  -- Better: Use SDBQL directly for roots.
  local result = Sdb:Sdbql("FOR p IN pages FILTER p.parent_id == null OR p.parent_id == '' SORT p.position ASC, p.title ASC RETURN p")
  local pages = {}
  if result and result.result then
    for _, data in ipairs(result.result) do
      table.insert(pages, Page:new(data))
    end
  end
  
  self.layout = "pages"
  self:render("pages/index", {
    current_user = current_user,
    pages = pages,
    db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  })
end

-- Show a specific page
function PagesController:show()
  if not self.params or not self.params.key then
    return self:redirect("/pages")
  end

  local current_user = get_current_user()
  local page = Page:find(self.params.key)
  
  if not page then
    return self:redirect("/pages")
  end
  
  -- Get children for sub-navigation
  local children = page:children()
  
  self.layout = "pages"
  self:render("pages/show", {
    current_user = current_user,
    page = page,
    current_page = page, -- For layout/sidebar highlighting
    children = children,
    breadcrumbs = page:breadcrumbs()
  })
end

-- Create page form (Modal or inline)
function PagesController:new_form()
  local current_user = get_current_user()
  -- Safeguard params
  local params = self.params or {}
  local parent_id = params.parent_id
  
  self.layout = false
  self:render("pages/_form_modal", {
    parent_id = parent_id,
    current_user = current_user
  })
end

-- Create page
function PagesController:create()
  local current_user = get_current_user()
  if not current_user then
     return self:render("errors/403", {}, { status = 403 })
  end
  
  local params = self.params or {}
  local title = params.title
  
  if not title or title == "" then
    return self:html('<div class="text-red-400 text-sm">Title is required</div>')
  end
  
  local page = Page:new({
    title = title,
    parent_id = (params.parent_id and params.parent_id ~= "") and params.parent_id or nil,
    icon = params.icon,
    content = "", -- Start empty
    created_by = current_user._key,
    updated_by = current_user._key,
    created_at = os.time(),
    updated_at = os.time()
  })
  
  if page:save() then
    if not page._key then
       return self:html('<div class="text-red-400 text-sm">Error: Page created but key is missing. Please ensure migrations are run (pages collection missing).</div>')
    end

    if self:is_htmx_request() then
      self:set_header("HX-Redirect", "/pages/" .. page._key)
      return self:html("") 
    end
    return self:redirect("/pages/" .. page._key)
  else
    -- Handle validation errors (simple fallback)
    return self:html('<div class="text-red-400 text-sm">Error creating page</div>')
  end
end

-- Edit page form (Metadata)
function PagesController:edit()
  local current_user = get_current_user()
  if not self.params or not self.params.key then return self:html("") end
  
  local page = Page:find(self.params.key)
  
  if not page then
    return self:html('<div class="text-red-400">Page not found</div>')
  end
  
  self.layout = false
  self:render("pages/_form_modal", {
    page = page,
    current_user = current_user
  })
end

-- Update page
function PagesController:update()
  local current_user = get_current_user()
  if not current_user then return self:json({ error = "Unauthorized" }, 403) end
  
  if not self.params or not self.params.key then return self:json({ error = "Missing key" }, 400) end

  local page = Page:find(self.params.key)
  
  if not page then
     return self:json({ error = "Page not found" }, 404)
  end
  
  local updates = { updated_at = os.time(), updated_by = current_user._key }
  local params = self.params
  
  if params.title then updates.title = params.title end
  if params.content then updates.content = params.content end
  if params.icon then updates.icon = params.icon end
  if params.cover then updates.cover = params.cover end
  
  page:update(updates)
  
  if self:is_htmx_request() then
    -- If updating content from editor, just return success
    if params.content then
      return self:json({ success = true })
    end
    
    -- If updating metadata, redirect to show page
     self:set_header("HX-Redirect", "/pages/" .. page._key)
     return self:html("")
  end
  
  return self:redirect("/pages/" .. page._key)
end

-- Delete page
function PagesController:destroy()
  if not self.params or not self.params.key then return self:redirect("/pages") end

  local page = Page:find(self.params.key)
  
  if page then
    page:destroy()
  end
  
  if self:is_htmx_request() then
    self:set_header("HX-Redirect", "/pages")
    return self:html("")
  end
  
  return self:redirect("/pages")
end

-- Sidebar partial (HTMX)
function PagesController:sidebar()
  -- Fetch root pages for the sidebar
  local result = Sdb:Sdbql("FOR p IN pages FILTER p.parent_id == null OR p.parent_id == '' SORT p.position ASC, p.title ASC RETURN p")
  local pages = {}
  if result and result.result then
    for _, data in ipairs(result.result) do
      table.insert(pages, Page:new(data))
    end
  end
  
  self.layout = false
  self:render("pages/_sidebar_tree", {
    pages = pages,
    current_page_key = self.params and self.params.current_page_key
  })
end

-- Load children for a sidebar item (Lazy loading)
function PagesController:sidebar_children()
  local params = self.params or {}
  local parent_id = params.parent_id
  local pages = Page:new():where({ parent_id = parent_id }):order("doc.position ASC, doc.title ASC"):all()
  
  self.layout = false
  self:render("pages/_sidebar_tree", {
    pages = pages,
    current_page_key = params.current_page_key
  })
end

-- Upload cover image
function PagesController:upload_cover()
  local file = self.params.file
  if not file then
    return self:json({ error = "No file uploaded" }, 400)
  end
  
  local key = "cover_" .. os.time() .. "_" .. math.random(1000)
  local ext = file.name:match("%.([^%.]+)$")
  if ext then key = key .. "." .. ext end
  
  local upload_dir = "public/uploads"
  os.execute("mkdir -p " .. upload_dir)
  
  local file_path = upload_dir .. "/" .. key
  local f = io.open(file_path, "wb")
  if f then
    f:write(file.content)
    f:close()
    return self:json({ url = "/uploads/" .. key })
  else
    return self:json({ error = "Failed to save file" }, 500)
  end
end

--------------------------------------------------------------------------------
-- Block CRUD Actions
--------------------------------------------------------------------------------

-- Get all blocks for a page (JSON)
function PagesController:blocks()
  if not self.params or not self.params.key then
    return self:json({ error = "Missing page key" }, 400)
  end
  
  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end
  
  return self:json({ blocks = page:get_blocks() })
end

-- Add a new block to a page
function PagesController:add_block()
  if not self.params or not self.params.key then
    return self:json({ error = "Missing page key" }, 400)
  end
  
  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end
  
  local block_type = self.params.type or "paragraph"
  local data = {}
  
  -- Handle block-type-specific data
  if block_type == "header" then
    data.level = tonumber(self.params.level) or 2
    data.content = self.params.content or ""
  elseif block_type == "paragraph" then
    data.content = self.params.content or ""
  elseif block_type == "code" then
    data.language = self.params.language or "lua"
    data.content = self.params.content or ""
  elseif block_type == "table" then
    -- Default 2x2 table
    data.data = self.params.data or {{"", ""}, {"", ""}}
  elseif block_type == "image" then
    data.url = self.params.url or ""
    data.caption = self.params.caption or ""
  elseif block_type == "file" then
    data.url = self.params.url or ""
    data.filename = self.params.filename or "file"
  end
  
  local after_id = self.params.after_id
  local new_block = page:add_block(block_type, data, after_id)
  
  return self:json({ block = new_block })
end

-- Update a specific block
function PagesController:update_block()
  if not self.params or not self.params.key or not self.params.block_id then
    return self:json({ error = "Missing parameters" }, 400)
  end
  
  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end
  
  local data = {}
  -- Allow updating any field passed in params (except id)
  for k, v in pairs(self.params) do
    if k ~= "key" and k ~= "block_id" and k ~= "_method" then
      data[k] = v
    end
  end
  
  local updated = page:update_block(self.params.block_id, data)
  if updated then
    return self:json({ block = updated })
  else
    return self:json({ error = "Block not found" }, 404)
  end
end

-- Delete a block
function PagesController:delete_block()
  if not self.params or not self.params.key or not self.params.block_id then
    return self:json({ error = "Missing parameters" }, 400)
  end
  
  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end
  
  if page:remove_block(self.params.block_id) then
    return self:json({ success = true })
  else
    return self:json({ error = "Block not found" }, 404)
  end
end

-- Reorder blocks
function PagesController:reorder_blocks()
  if not self.params or not self.params.key then
    return self:json({ error = "Missing page key" }, 400)
  end
  
  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end
  
  -- Parse ordered IDs (expect JSON array)
  local ordered_ids = self.params.order
  if type(ordered_ids) == "string" then
    local ok, parsed = pcall(DecodeJson, ordered_ids)
    if ok then ordered_ids = parsed end
  end
  
  if type(ordered_ids) ~= "table" then
    return self:json({ error = "Invalid order format" }, 400)
  end
  
  local blocks = page:reorder_blocks(ordered_ids)
  return self:json({ blocks = blocks })
end

-- Generic file upload (for Image/File blocks)
function PagesController:upload_file()
  local file = self.params.file
  if not file then
    return self:json({ error = "No file uploaded" }, 400)
  end
  
  local key = "file_" .. os.time() .. "_" .. math.random(10000)
  local ext = file.name:match("%.([^%.]+)$")
  if ext then key = key .. "." .. ext end
  
  local upload_dir = "public/uploads"
  os.execute("mkdir -p " .. upload_dir)
  
  local file_path = upload_dir .. "/" .. key
  local f = io.open(file_path, "wb")
  if f then
    f:write(file.content)
    f:close()
    return self:json({ url = "/uploads/" .. key, filename = file.name })
  else
    return self:json({ error = "Failed to save file" }, 500)
  end
end

return PagesController
