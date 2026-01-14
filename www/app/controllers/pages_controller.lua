local Controller = require("controller")
local PagesController = Controller:extend()
local Page = require("models.page")
local PageRevision = require("models.page_revision")
local AuthHelper = require("helpers.auth_helper")

local function get_current_user()
  return AuthHelper.get_current_user()
end

-- Index: Dashboard / Root pages
function PagesController:index()
  local current_user = get_current_user()
  
  -- Fetch root pages (pages without a parent)
  local result = Sdb:Sdbql("FOR p IN pages FILTER (p.parent_id == null OR p.parent_id == '') SORT p.position ASC, p.title ASC RETURN p")
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

  -- Get blocks for content editor
  local blocks = page:get_blocks()

  self.layout = "pages"
  self:render("pages/show", {
    current_user = current_user,
    page = page,
    blocks = blocks,
    current_page = page, -- For layout/sidebar highlighting
    children = children,
    breadcrumbs = page:breadcrumbs()
  })
end

-- Create page form (Modal or inline)
function PagesController:new_form()
  local current_user = get_current_user()
  -- Safeguard params - check both self.params and GetParam for query string
  local params = self.params or {}
  local parent_id = params.parent_id or GetParam("parent_id")

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

  -- Get parent_id - ensure it's a string or nil
  local parent_id = params.parent_id
  if parent_id == "" or parent_id == "null" or parent_id == "undefined" then
    parent_id = nil
  end

  local page = Page:new({
    title = title,
    parent_id = parent_id,
    icon = params.icon,
    cover = (params.cover and params.cover ~= "") and params.cover or nil,
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

    -- Record creation in history
    PageRevision.record(page, "create", current_user, {
      after = { title = page.title, icon = page.icon },
      summary = "Created page"
    })

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

  -- Get all pages for parent selection (exclude descendants to prevent circular refs)
  local result = Sdb:Sdbql("FOR p IN pages SORT p.title ASC RETURN p")
  local all_pages = {}
  if result and result.result then
    for _, data in ipairs(result.result) do
      local p = Page:new(data)
      -- Exclude current page and its descendants
      if p._key ~= page._key and not p:is_descendant_of(page._key) then
        table.insert(all_pages, p)
      end
    end
  end

  self.layout = false
  self:render("pages/_form_modal", {
    page = page,
    all_pages = all_pages,
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

  -- Capture state before changes for history
  local before_state = {
    title = page.title,
    icon = page.icon,
    cover = page.cover,
    parent_id = page.parent_id
  }

  local updates = { updated_at = os.time(), updated_by = current_user._key }
  local params = self.params
  local changed_fields = {}

  if params.title and params.title ~= page.title then
    updates.title = params.title
    table.insert(changed_fields, "title")
  end
  if params.content then
    updates.content = params.content
    table.insert(changed_fields, "content")
  end
  if params.icon and params.icon ~= page.icon then
    updates.icon = params.icon
    table.insert(changed_fields, "icon")
  end
  if params.cover and params.cover ~= page.cover then
    updates.cover = params.cover
    table.insert(changed_fields, "cover")
  end
  -- Handle parent_id - empty string means root level (no parent)
  if params.parent_id ~= nil then
    local new_parent = (params.parent_id ~= "") and params.parent_id or nil
    if new_parent ~= page.parent_id then
      updates.parent_id = new_parent
      table.insert(changed_fields, "parent")
    end
  end

  page:update(updates)

  -- Record in history if something changed
  if #changed_fields > 0 then
    local summary = "Updated " .. table.concat(changed_fields, ", ")
    PageRevision.record(page, "update", current_user, {
      before = before_state,
      after = updates,
      field = table.concat(changed_fields, ","),
      summary = summary
    })
  end
  
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
  local current_user = get_current_user()
  if not self.params or not self.params.key then return self:redirect("/pages") end

  local page = Page:find(self.params.key)

  if page then
    -- Record deletion in history before destroying
    PageRevision.record(page, "delete", current_user, {
      before = {
        title = page.title,
        icon = page.icon,
        content = page:get_blocks()
      },
      summary = "Deleted page: " .. page.title
    })

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
  -- Fetch root pages for the sidebar (pages without a parent)
  local result = Sdb:Sdbql("FOR p IN pages FILTER (p.parent_id == null OR p.parent_id == '') SORT p.position ASC, p.title ASC RETURN p")
  local pages = {}
  if result and result.result then
    for _, data in ipairs(result.result) do
      table.insert(pages, Page:new(data))
    end
  end

  self.layout = false
  self:render("pages/_sidebar_tree", {
    pages = pages,
    current_page_key = self.params and self.params.current_page_key,
    is_root = true
  })
end

-- Load children for a sidebar item (Lazy loading)
function PagesController:sidebar_children()
  local params = self.params or {}
  local parent_id = params.parent_id

  -- Use direct SDBQL for more reliable querying
  local result = Sdb:Sdbql(
    "FOR p IN pages FILTER p.parent_id == @parent_id SORT p.position ASC, p.title ASC RETURN p",
    { parent_id = parent_id }
  )

  local pages = {}
  if result and result.result then
    for _, data in ipairs(result.result) do
      table.insert(pages, Page:new(data))
    end
  end

  self.layout = false
  self:render("pages/_sidebar_tree", {
    pages = pages,
    current_page_key = params.current_page_key or GetParam("current_page_key")
  })
end

-- Toggle favorite status
function PagesController:toggle_favorite()
  local page_id = self.params and self.params.key
  if not page_id then
    return self:json({ error = "Missing page id" }, 400)
  end

  local page = Page:find(page_id)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  local is_favorite = not page.is_favorite
  page:update({ is_favorite = is_favorite })

  return self:json({ success = true, is_favorite = is_favorite })
end

-- Get favorite pages
function PagesController:favorites()
  local result = Sdb:Sdbql("FOR p IN pages FILTER p.is_favorite == true SORT p.title ASC RETURN p")
  local pages = {}
  if result and result.result then
    for _, data in ipairs(result.result) do
      table.insert(pages, Page:new(data))
    end
  end

  self.layout = false
  self:render("pages/_favorites", {
    pages = pages,
    current_page_key = self.params and self.params.current_page_key
  })
end

-- Reorder pages (drag & drop in sidebar)
function PagesController:reorder_pages()
  local params = self.params or {}
  local page_id = params.page_id
  local parent_id = params.parent_id
  local sibling_order = params.sibling_order

  if not page_id then
    return self:json({ error = "Missing page_id" }, 400)
  end

  local page = Page:find(page_id)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  -- Update parent_id (move to new parent or root)
  -- Use empty string for root level (nil doesn't persist in Lua tables)
  local new_parent = (parent_id and parent_id ~= "") and parent_id or ""
  page:update({ parent_id = new_parent })

  -- Update positions for all siblings
  if sibling_order and type(sibling_order) == "table" then
    for i, sib_id in ipairs(sibling_order) do
      local sib = Page:find(sib_id)
      if sib then
        sib:update({ position = i * 1000 }) -- Use large increments for future insertions
      end
    end
  end

  return self:json({ success = true })
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
  local current_user = get_current_user()
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
    data.columnWidths = {150, 150}
  elseif block_type == "image" then
    data.url = self.params.url or ""
    data.caption = self.params.caption or ""
  elseif block_type == "file" then
    data.url = self.params.url or ""
    data.filename = self.params.filename or "file"
  elseif block_type == "ai" then
    data.prompt = self.params.prompt or ""
    data.provider = self.params.provider or "default"
    data.status = "idle"
    data.generated_blocks = {}
  end

  local after_id = self.params.after_id
  local new_block = page:add_block(block_type, data, after_id)

  -- Record in history
  PageRevision.record(page, "update", current_user, {
    after = { block = new_block },
    field = "blocks",
    summary = "Added " .. block_type .. " block"
  })

  -- Return HTML partial for HTMX
  if self:is_htmx_request() then
    self.layout = false
    return self:render("pages/_block", {
      block = new_block,
      page_key = self.params.key,
      editing = true
    })
  end

  return self:json({ block = new_block })
end

-- Update a specific block
function PagesController:update_block()
  local current_user = get_current_user()
  if not self.params or not self.params.key or not self.params.block_id then
    return self:json({ error = "Missing parameters" }, 400)
  end

  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  -- Get block before update for history
  local blocks = page:get_blocks()
  local block_before = nil
  for _, b in ipairs(blocks) do
    if b.id == self.params.block_id then
      block_before = b
      break
    end
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
    -- Record in history
    PageRevision.record(page, "update", current_user, {
      before = { block = block_before },
      after = { block = updated },
      field = "blocks",
      summary = "Updated " .. (updated.type or "block")
    })
    return self:json({ block = updated })
  else
    return self:json({ error = "Block not found" }, 404)
  end
end

-- Delete a block
function PagesController:delete_block()
  local current_user = get_current_user()
  if not self.params or not self.params.key or not self.params.block_id then
    return self:json({ error = "Missing parameters" }, 400)
  end

  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  -- Get block before deletion for history
  local blocks = page:get_blocks()
  local block_before = nil
  for _, b in ipairs(blocks) do
    if b.id == self.params.block_id then
      block_before = b
      break
    end
  end

  if page:remove_block(self.params.block_id) then
    -- Record in history
    if block_before then
      PageRevision.record(page, "update", current_user, {
        before = { block = block_before },
        field = "blocks",
        summary = "Deleted " .. (block_before.type or "block")
      })
    end

    -- Return empty for HTMX (block will be removed from DOM)
    if self:is_htmx_request() then
      return self:html("")
    end
    return self:json({ success = true })
  else
    return self:json({ error = "Block not found" }, 404)
  end
end

-- Get a single block (view mode)
function PagesController:get_block()
  if not self.params or not self.params.key or not self.params.block_id then
    return self:json({ error = "Missing parameters" }, 400)
  end

  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  local blocks = page:get_blocks()
  local block = nil
  for _, b in ipairs(blocks) do
    if b.id == self.params.block_id then
      block = b
      break
    end
  end

  if not block then
    return self:json({ error = "Block not found" }, 404)
  end

  self.layout = false
  return self:render("pages/_block", {
    block = block,
    page_key = self.params.key,
    editing = false
  })
end

-- Get a single block (edit mode)
function PagesController:get_block_edit()
  if not self.params or not self.params.key or not self.params.block_id then
    return self:json({ error = "Missing parameters" }, 400)
  end

  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  local blocks = page:get_blocks()
  local block = nil
  for _, b in ipairs(blocks) do
    if b.id == self.params.block_id then
      block = b
      break
    end
  end

  if not block then
    return self:json({ error = "Block not found" }, 404)
  end

  self.layout = false
  return self:render("pages/_block", {
    block = block,
    page_key = self.params.key,
    editing = true
  })
end

-- Add comment to a block
function PagesController:add_block_comment()
  if not self.params or not self.params.key or not self.params.block_id then
    return self:json({ error = "Missing parameters" }, 400)
  end

  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  local blocks = page:get_blocks()
  local block = nil
  local block_index = nil
  for i, b in ipairs(blocks) do
    if b.id == self.params.block_id then
      block = b
      block_index = i
      break
    end
  end

  if not block then
    return self:json({ error = "Block not found" }, 404)
  end

  -- Add comment
  if not block.comments then block.comments = {} end
  table.insert(block.comments, {
    text = self.params.text,
    author = self.params.author,
    date = self.params.date
  })

  -- Update block
  page:update_block(self.params.block_id, { comments = block.comments })

  return self:json({
    comments = block.comments,
    block_type = block.type
  })
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

--------------------------------------------------------------------------------
-- AI Block Generation
--------------------------------------------------------------------------------

-- Helper to make authenticated API calls to SoliDB backend
function PagesController:fetch_api(path, options)
  -- Use global Sdb connection which is already authenticated
  local db = Sdb
  if not db then
    return nil, nil, nil
  end

  -- Refresh token if needed
  db:RefreshToken()

  local server_url = db._db_config.url
  local token = db._token

  -- Ensure no double slashes
  if server_url:sub(-1) == "/" then server_url = server_url:sub(1, -2) end
  if path:sub(1, 1) ~= "/" then path = "/" .. path end

  options = options or {}
  options.headers = options.headers or {}
  options.headers["Authorization"] = "Bearer " .. (token or "")
  options.headers["Content-Type"] = "application/json"

  local full_url = server_url .. path
  local status, headers, body = Fetch(full_url, options)

  return status, headers, body
end

-- Get database from config
function PagesController:get_db()
  if Sdb and Sdb._db_config and Sdb._db_config.db_name then
    return Sdb._db_config.db_name
  end
  return "_system"
end

-- Generate AI content for a block
function PagesController:generate_ai()
  if not self.params or not self.params.key or not self.params.block_id then
    return self:json({ error = "Missing parameters" }, 400)
  end

  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  local blocks = page:get_blocks()
  local block = nil
  for _, b in ipairs(blocks) do
    if b.id == self.params.block_id then
      block = b
      break
    end
  end

  if not block then
    return self:json({ error = "Block not found" }, 404)
  end

  local prompt = self.params.prompt or block.prompt
  local provider = self.params.provider or block.provider or "default"

  if not prompt or prompt == "" then
    return self:json({ error = "Prompt is required" }, 400)
  end

  -- Build system prompt for structured content generation
  local system_prompt = "You generate structured content. Return a JSON array where each element has a 'type' field.\n" ..
    "Supported types:\n" ..
    '- header: { "type": "header", "level": 1-3, "content": "text" }\n' ..
    '- paragraph: { "type": "paragraph", "content": "html text" }\n' ..
    '- code: { "type": "code", "language": "lua/js/python/etc", "content": "code" }\n' ..
    '- table: { "type": "table", "data": [["H1","H2"],["r1","r2"]] }\n' ..
    '- list: { "type": "list", "items": ["item1", "item2"] }\n' ..
    "Return ONLY valid JSON array, no markdown or explanation."

  -- Call the AI endpoint
  local db = self:get_db()
  local status, headers, body = self:fetch_api("/_api/database/" .. db .. "/ai/generate", {
    method = "POST",
    body = EncodeJson({
      prompt = prompt,
      system = system_prompt,
      provider = provider
    })
  })

  if not status or status ~= 200 then
    local err_msg = "AI generation failed"
    if body then
      local ok, parsed = pcall(DecodeJson, body)
      if ok and parsed then
        if parsed.error then
          err_msg = parsed.error
        elseif parsed.message then
          err_msg = parsed.message
        else
          err_msg = body
        end
      else
        err_msg = body or ("HTTP " .. tostring(status or "no response"))
      end
    else
      err_msg = "No response from AI service (status: " .. tostring(status) .. ")"
    end
    return self:json({ error = err_msg }, status or 500)
  end

  local response_data = {}
  if body then
    local ok, parsed = pcall(DecodeJson, body)
    if ok and parsed then
      response_data = parsed
    end
  end

  -- Parse the AI response to extract generated content
  local generated_items = {}
  local content = response_data.content

  if type(content) == "string" then
    -- Strip markdown code blocks if present (```json ... ``` or ``` ... ```)
    local stripped = content:gsub("^%s*```[%w]*%s*", ""):gsub("%s*```%s*$", "")
    -- Also handle backticks without language specifier
    stripped = stripped:gsub("^%s*`+%s*", ""):gsub("%s*`+%s*$", "")
    stripped = stripped:match("^%s*(.-)%s*$") or stripped  -- Trim whitespace

    -- Try to parse as JSON array
    local ok, parsed_content = pcall(DecodeJson, stripped)
    if ok and type(parsed_content) == "table" then
      generated_items = parsed_content
    else
      -- Wrap plain text as a paragraph
      generated_items = {{ type = "paragraph", content = content }}
    end
  elseif type(content) == "table" then
    generated_items = content
  end

  -- Create actual editable blocks for each generated item
  local created_blocks = {}
  local after_id = self.params.block_id  -- Insert after the AI block

  for _, item in ipairs(generated_items) do
    local block_type = item.type or "paragraph"
    local block_data = {}

    if block_type == "header" then
      block_data.level = item.level or 2
      block_data.content = item.content or ""
    elseif block_type == "paragraph" then
      block_data.content = item.content or ""
    elseif block_type == "code" then
      block_data.language = item.language or "plaintext"
      block_data.content = item.content or ""
    elseif block_type == "table" then
      block_data.data = item.data or {{"", ""}, {"", ""}}
      block_data.columnWidths = {}
      if block_data.data[1] then
        for i = 1, #block_data.data[1] do
          table.insert(block_data.columnWidths, 150)
        end
      end
    elseif block_type == "list" then
      -- Convert list to paragraph with bullet points
      block_type = "paragraph"
      local items_html = ""
      if item.items then
        for _, li in ipairs(item.items) do
          items_html = items_html .. "• " .. li .. "<br>"
        end
      end
      block_data.content = items_html
    else
      -- Default to paragraph for unknown types
      block_type = "paragraph"
      block_data.content = item.content or ""
    end

    -- Create the block
    local new_block = page:add_block(block_type, block_data, after_id)
    table.insert(created_blocks, new_block)
    after_id = new_block.id  -- Chain the blocks
  end

  -- Remove the AI block after generating content
  page:remove_block(self.params.block_id)

  return self:json({
    success = true,
    created_blocks = created_blocks
  })
end

-- Get available AI providers
function PagesController:ai_providers()
  return self:json({
    providers = {
      { id = "default", name = "Default" },
      { id = "ollama", name = "Ollama (Local)" },
      { id = "openai", name = "OpenAI" },
      { id = "anthropic", name = "Anthropic" },
      { id = "gemini", name = "Gemini" }
    }
  })
end

--------------------------------------------------------------------------------
-- Page History
--------------------------------------------------------------------------------

-- Get page history
function PagesController:history()
  if not self.params or not self.params.key then
    return self:json({ error = "Missing page key" }, 400)
  end

  local page = Page:find(self.params.key)
  if not page then
    return self:json({ error = "Page not found" }, 404)
  end

  local revisions = PageRevision.history_for(self.params.key, 100)

  self.layout = false
  return self:render("pages/_history", {
    page = page,
    revisions = revisions
  })
end

-- Get a specific revision detail
function PagesController:revision()
  if not self.params or not self.params.revision_id then
    return self:json({ error = "Missing revision id" }, 400)
  end

  local revision = PageRevision:find(self.params.revision_id)
  if not revision then
    return self:json({ error = "Revision not found" }, 404)
  end

  self.layout = false
  return self:render("pages/_revision_detail", {
    revision = revision
  })
end

return PagesController
