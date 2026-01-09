local Model = require("model")
local GitHelper = require("helpers.git_helper")

local Repository = Model.create("repositories", {
  permitted_fields = { "name", "description", "is_public", "collaborators" },
  validations = {
    name = { presence = true, length = { between = {3, 50} }, format = "^[a-zA-Z0-9_%-]+$" },
    owner_id = { presence = true }
  },
  -- Register callbacks by method name
  after_create = { "init_git_repo" }
})

-- After create callback - init the bare git repo
function Repository:init_git_repo()
  if self.name or (self.data and self.data.name) then
    local name = self.name or self.data.name
    GitHelper.init_repo(name)
  end
end

-- Delete the physical git repo files
function Repository:delete_repo()
  if self.name or (self.data and self.data.name) then
    local name = self.name or self.data.name
    local repo_path = GitHelper.get_repo_path(name)
    os.execute(string.format("rm -rf '%s'", repo_path))
  end
end

-- Check if a user has access to this repository
function Repository:user_has_access(user_key)
  if not user_key then return false end

  local owner_id = self.owner_id or (self.data and self.data.owner_id)
  local is_public = self.is_public or (self.data and self.data.is_public)
  local collaborators = self.collaborators or (self.data and self.data.collaborators) or {}

  -- Owner always has access
  if owner_id == user_key then
    return true
  end

  -- Public repos allow read access to everyone
  if is_public then
    return true
  end

  -- Check if user is a collaborator
  for _, collab_key in ipairs(collaborators) do
    if collab_key == user_key then
      return true
    end
  end

  return false
end

-- Check if user can push (owner or collaborator, not just public read)
function Repository:user_can_push(user_key)
  if not user_key then return false end

  local owner_id = self.owner_id or (self.data and self.data.owner_id)
  local collaborators = self.collaborators or (self.data and self.data.collaborators) or {}

  -- Owner can push
  if owner_id == user_key then
    return true
  end

  -- Collaborators can push
  for _, collab_key in ipairs(collaborators) do
    if collab_key == user_key then
      return true
    end
  end

  return false
end

-- Add a collaborator
function Repository:add_collaborator(user_key)
  local collaborators = self.collaborators or (self.data and self.data.collaborators) or {}

  -- Check if already a collaborator
  for _, collab_key in ipairs(collaborators) do
    if collab_key == user_key then
      return false -- Already exists
    end
  end

  table.insert(collaborators, user_key)
  self:update({ collaborators = collaborators })
  return true
end

-- Remove a collaborator
function Repository:remove_collaborator(user_key)
  local collaborators = self.collaborators or (self.data and self.data.collaborators) or {}
  local new_collaborators = {}

  for _, collab_key in ipairs(collaborators) do
    if collab_key ~= user_key then
      table.insert(new_collaborators, collab_key)
    end
  end

  self:update({ collaborators = new_collaborators })
  return true
end

-- Get collaborators with user info
function Repository:collaborators_with_info()
  local collaborators = self.collaborators or (self.data and self.data.collaborators) or {}
  if #collaborators == 0 then return {} end
  
  -- Sanitize keys (ensure they are strings)
  local keys = {}
  for _, c in ipairs(collaborators) do
    if type(c) == "table" and c.user_key then
      table.insert(keys, c.user_key)
    elseif type(c) == "string" then
      table.insert(keys, c)
    end
  end
  
  if #keys == 0 then return {} end

  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER u._key IN @keys
    RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }
  ]], { keys = keys })

  return (result and result.result) or {}
end

-- Find repository by name
function Repository.find_by_name(name)
  local result = Sdb:Sdbql([[
    FOR r IN repositories
    FILTER r.name == @name
    LIMIT 1
    RETURN r
  ]], { name = name })

  if result and result.result and result.result[1] then
    local repo = Repository:new()
    repo.data = result.result[1]
    repo._key = result.result[1]._key
    repo._id = result.result[1]._id
    -- Copy fields to top level for convenience
    for k, v in pairs(result.result[1]) do
      repo[k] = v
    end
    return repo
  end
  return nil
end

return Repository
