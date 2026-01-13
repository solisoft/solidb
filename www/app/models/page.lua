local Model = require("model")

local Page = Model.create("pages", {
  permitted_fields = { "title", "content", "parent_id", "position", "icon", "cover", "created_by", "updated_by" },
  validations = {
    title = { presence = true, length = { between = {1, 200} } }
  },
  before_create = { "set_order" }
})

-- Callback to set default position
function Page.set_order(data)
  if not data.position then
    data.position = os.time()
  end
  return data
end

-- Get parent page
function Page:parent()
  local parent_id = self.parent_id or (self.data and self.data.parent_id)
  if not parent_id or parent_id == "" then return nil end
  return Page:find(parent_id)
end

-- Get child pages
function Page:children()
  return Page:new():where({ parent_id = self._key }):order("doc.position ASC, doc.title ASC"):all()
end

-- Get breadcrumbs (path to root)
function Page:breadcrumbs()
  local crumbs = {}
  local current = self
  
  -- Prevent infinite loops
  local depth = 0
  while current and depth < 20 do
    table.insert(crumbs, 1, current)
    current = current:parent()
    depth = depth + 1
  end
  
  return crumbs
end

-- Check if page is descendant of another page
function Page:is_descendant_of(page_key)
  local current = self
  local depth = 0
  while current and depth < 20 do
    local parent = current:parent()
    if not parent then return false end
    if parent._key == page_key then return true end
    current = parent
    depth = depth + 1
  end
  return false
end

-- Get user who created the page
function Page:creator()
  local created_by = self.created_by or (self.data and self.data.created_by)
  if not created_by then return nil end
  local User = require("models.user")
  return User:find(created_by)
end

return Page
