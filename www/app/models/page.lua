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

--------------------------------------------------------------------------------
-- Block Management
--------------------------------------------------------------------------------

-- Generate unique block ID
local function generate_block_id()
  return string.format("%x%x", os.time(), math.random(10000, 99999))
end

-- Get blocks (content is now an array of blocks)
function Page:get_blocks()
  local content = self.content or (self.data and self.data.content)
  if type(content) == "string" then
    -- Migrate: old string content becomes a single paragraph block
    if content == "" then return {} end
    return {{ id = generate_block_id(), type = "paragraph", content = content }}
  end
  return content or {}
end

-- Add a new block
function Page:add_block(block_type, data, after_id)
  local blocks = self:get_blocks()
  local new_block = {
    id = generate_block_id(),
    type = block_type,
  }
  
  -- Merge block-type-specific data
  for k, v in pairs(data or {}) do
    new_block[k] = v
  end
  
  -- Insert at position
  if after_id then
    for i, block in ipairs(blocks) do
      if block.id == after_id then
        table.insert(blocks, i + 1, new_block)
        self:update({ content = blocks })
        return new_block
      end
    end
  end
  
  -- Default: append at end
  table.insert(blocks, new_block)
  self:update({ content = blocks })
  return new_block
end

-- Update a specific block
function Page:update_block(block_id, data)
  local blocks = self:get_blocks()
  for i, block in ipairs(blocks) do
    if block.id == block_id then
      for k, v in pairs(data) do
        blocks[i][k] = v
      end
      self:update({ content = blocks })
      return blocks[i]
    end
  end
  return nil
end

-- Remove a block
function Page:remove_block(block_id)
  local blocks = self:get_blocks()
  for i, block in ipairs(blocks) do
    if block.id == block_id then
      table.remove(blocks, i)
      self:update({ content = blocks })
      return true
    end
  end
  return false
end

-- Reorder blocks (receives array of block IDs in new order)
function Page:reorder_blocks(ordered_ids)
  local blocks = self:get_blocks()
  local block_map = {}
  for _, block in ipairs(blocks) do
    block_map[block.id] = block
  end
  
  local new_blocks = {}
  for _, id in ipairs(ordered_ids) do
    if block_map[id] then
      table.insert(new_blocks, block_map[id])
    end
  end
  
  self:update({ content = new_blocks })
  return new_blocks
end

return Page
