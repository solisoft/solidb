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
  
  -- Generate unique key
  local key = "cover_" .. os.time() .. "_" .. math.random(1000)
  local ext = file.name:match("%.([^%.]+)$")
  if ext then key = key .. "." .. ext end
  
  -- Use Sdb to save blob (assuming helper or direct call)
  -- Since we don't have explicit blob API in context, let's try to assume we can write to public folder 
  -- OR better, use the DB's blob storage if we knew how.
  -- Given the user context, let's try to save to `public/uploads` if possible, or use the `_uploads` collection.
  
  -- Try to save to `_uploads` collection using SDBQL? No, BLOBs are special.
  -- Let's use `IO` to save to `www/public/uploads` for simplicity in this MVP.
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

return PagesController
