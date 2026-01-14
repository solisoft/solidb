local Controller = require("controller")
local RepositoriesController = Controller:extend()
local Repository = require("models.repository")
local GitHelper = require("helpers.git_helper")
local GitSync = require("helpers.git_sync")
local AuthHelper = require("helpers.auth_helper")
local DateHelper = require("helpers.date_helper")

-- Make helper available to views
_G.time_ago_in_words = DateHelper.time_ago_in_words

-- Get current user (middleware ensures user is authenticated)
local function get_current_user()
  return AuthHelper.get_current_user()
end

function RepositoriesController:before_action()
  self.layout = "talks"
  self.current_user = get_current_user()
end

function RepositoriesController:index()
  local current_user = get_current_user()
  local repos = Repository.where({ owner_id = current_user._key }):all()
  local MergeRequest = require("models.merge_request")

  -- Decorate repositories with extra stats
  for _, repo in ipairs(repos) do
    -- Get last commit info
    local last_commit = GitHelper.get_last_commit(repo.name, "HEAD")
    if last_commit then
      repo.last_activity_at = last_commit.timestamp
      repo.last_commit_message = last_commit.message
    end

    -- Get open MR count
    local open_mrs = 0
    local mrs = MergeRequest.where({ repo_id = repo.id }):all()
    for _, mr in ipairs(mrs) do
      if mr.status == "open" then open_mrs = open_mrs + 1 end
    end
    repo.open_mrs_count = open_mrs

    -- Get default branch
    repo.default_branch = GitHelper.get_default_branch(repo.name) or "master"
  end

  table.sort(repos, function(a, b)
    -- Sort by last activity (or ID as fallback)
    local a_time = a.last_activity_at or a.created_at or 0
    local b_time = b.last_activity_at or b.created_at or 0
    return a_time > b_time
  end)

  self:render("repositories/index", { repositories = repos })
end

function RepositoriesController:new_form()
  self:render("repositories/new", { repository = Repository:new() })
end

function RepositoriesController:create()
  local current_user = get_current_user()
  local params = Repository.permit(self.params.repository)
  params.owner_id = current_user._key

  local repository = Repository:new(params)

  -- Check if name is taken physically or in DB?
  -- Assuming DB unique index handles it, or GitHelper fails.
  if GitHelper.repo_exists(repository.name) then
    repository.errors = { name = "Repository name already exists" }
    return self:render("repositories/new", { repository = repository })
  end

  if repository:save() then
    -- after_create hook in model inits the repo
    self:redirect("/repositories")
  else
    self:render("repositories/new", { repository = repository })
  end
end

function RepositoriesController:show()
  local id = self.params.id
  -- Redirect to tree view (code browser) as default
  return self:redirect("/repositories/" .. id .. "/tree")
end

-- Commits view
function RepositoriesController:commits()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Get latest commits
  local commits = GitHelper.get_commits(repository.name, "HEAD", 50)

  -- Get merge requests count for badge
  local MergeRequest = require("models.merge_request")
  local mrs = MergeRequest.where({ repo_id = repository.id }):all()
  local open_mrs = 0
  for _, mr in ipairs(mrs) do
    if mr.status == "open" then open_mrs = open_mrs + 1 end
  end

  -- Get branches for selector
  local branches = GitHelper.get_branches(repository.name)
  local ref = self.params.ref or GitHelper.get_default_branch(repository.name) or "HEAD"

  self:render("repositories/commits", {
    repository = repository,
    commits = commits,
    merge_requests = mrs,
    open_mrs_count = open_mrs,
    branches = branches,
    ref = ref
  })
end

function RepositoriesController:edit()
  local current_user = get_current_user()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership
  if repository.owner_id ~= current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  self:render("repositories/edit", { repository = repository })
end

function RepositoriesController:update()
  local current_user = get_current_user()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership
  if repository.owner_id ~= current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  local params = Repository.permit(self.params.repository)
  -- Don't allow changing name (would break git repo)
  params.name = nil

  for k, v in pairs(params) do
    repository[k] = v
  end

  if repository:save() then
    self:redirect("/repositories/" .. id)
  else
    self:render("repositories/edit", { repository = repository })
  end
end

function RepositoriesController:destroy()
  local current_user = get_current_user()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership
  if repository.owner_id ~= current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  -- Delete blobs from storage
  GitSync.delete(repository.name)

  -- Delete physical repo files
  local repo_path = GitHelper.get_repo_path(repository.name)
  os.execute(string.format("rm -rf '%s'", repo_path))

  -- Delete from database
  repository:destroy()

  self:redirect("/repositories")
end

-- Restore repository from blob storage (if local folder is missing)
function RepositoriesController:restore()
  local current_user = get_current_user()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership
  if repository.owner_id ~= current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  -- Check if folder is missing
  if GitHelper.repo_exists(repository.name) then
    -- Folder exists, nothing to restore
    self:redirect("/repositories/" .. id)
    return
  end

  -- Restore from blob storage
  local ok, msg = GitSync.pull(repository.name)

  if ok then
    self:redirect("/repositories/" .. id)
  else
    self:render("repositories/restore_error", {
      repository = repository,
      error = msg
    })
  end
end

-- Manual sync to blob storage
function RepositoriesController:sync()
  local current_user = get_current_user()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:json({ error = "Repository not found" }, 404)
  end

  -- Check ownership
  if repository.owner_id ~= current_user._key then
    return self:json({ error = "Forbidden" }, 403)
  end

  local ok, msg = GitSync.push(repository.name)

  if ok then
    self:json({ success = true, message = msg })
  else
    self:json({ success = false, error = msg }, 500)
  end
end

-- Get sync status
function RepositoriesController:sync_status()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:json({ error = "Repository not found" }, 404)
  end

  local status = GitSync.status(repository.name)
  self:json(status)
end

-- Code Browser: Tree view (directory listing)
function RepositoriesController:tree()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Get path from splat (wildcard)
  local tree_path = self.params.path or ""
  -- Get branch from query param or use default
  local ref = self.params.ref or GitHelper.get_default_branch(repository.name) or "HEAD"

  -- Get tree entries with commit info
  local entries = GitHelper.get_tree(repository.name, ref, tree_path, true)

  -- Get branches for branch selector
  local branches = GitHelper.get_branches(repository.name)

  -- Build breadcrumbs
  local breadcrumbs = {}
  if tree_path and tree_path ~= "" then
    local parts = {}
    for part in tree_path:gmatch("[^/]+") do
      table.insert(parts, part)
      table.insert(breadcrumbs, {
        name = part,
        path = table.concat(parts, "/")
      })
    end
  end

  -- Get last commit for each entry (optional, can be slow for large dirs)
  -- For now, just get the repo's last commit
  local last_commit = GitHelper.get_last_commit(repository.name, ref, tree_path ~= "" and tree_path or nil)

  -- Get merge requests count for badge
  local MergeRequest = require("models.merge_request")
  local mrs = MergeRequest.where({ repo_id = repository.id }):all()
  local open_mrs = 0
  for _, mr in ipairs(mrs) do
    if mr.status == "open" then open_mrs = open_mrs + 1 end
  end

  self:render("repositories/tree", {
    repository = repository,
    entries = entries,
    tree_path = tree_path,
    ref = ref,
    branches = branches,
    breadcrumbs = breadcrumbs,
    last_commit = last_commit,
    open_mrs_count = open_mrs
  })
end

-- Code Browser: Blob view (file content)
function RepositoriesController:blob()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:render("errors/404", {}, { status = 404 })
  end

  local file_path = self.params.path or ""
  local ref = self.params.ref or GitHelper.get_default_branch(repository.name) or "HEAD"

  -- Check if path exists and is a file
  local path_type = GitHelper.get_path_type(repository.name, ref, file_path)
  if path_type ~= "blob" then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Get file content
  local content = GitHelper.get_file_content(repository.name, ref, file_path)
  local file_size = GitHelper.get_file_size(repository.name, ref, file_path)

  -- Get branches for branch selector
  local branches = GitHelper.get_branches(repository.name)

  -- Build breadcrumbs
  local breadcrumbs = {}
  local parts = {}
  for part in file_path:gmatch("[^/]+") do
    table.insert(parts, part)
    table.insert(breadcrumbs, {
      name = part,
      path = table.concat(parts, "/")
    })
  end

  -- Get file name and extension for syntax highlighting
  local file_name = file_path:match("([^/]+)$") or file_path
  local extension = file_name:match("%.([^%.]+)$") or ""

  -- Determine if binary
  local is_binary = content and content:find("\0") ~= nil
  local is_image = extension:match("^(png|jpg|jpeg|gif|svg|webp|ico|bmp)$")

  -- Get last commit for this file
  local last_commit = GitHelper.get_last_commit(repository.name, ref, file_path)

  -- Line count
  local line_count = 0
  if content and not is_binary then
    for _ in content:gmatch("\n") do
      line_count = line_count + 1
    end
    line_count = line_count + 1
  end

  -- Get merge requests count for badge
  local MergeRequest = require("models.merge_request")
  local mrs = MergeRequest.where({ repo_id = repository.id }):all()
  local open_mrs = 0
  for _, mr in ipairs(mrs) do
    if mr.status == "open" then open_mrs = open_mrs + 1 end
  end

  self:render("repositories/blob", {
    repository = repository,
    file_path = file_path,
    file_name = file_name,
    extension = extension,
    content = content,
    file_size = file_size,
    line_count = line_count,
    is_binary = is_binary,
    is_image = is_image,
    ref = ref,
    branches = branches,
    breadcrumbs = breadcrumbs,
    last_commit = last_commit,
    open_mrs_count = open_mrs
  })
end

-- Raw file content
function RepositoriesController:raw()
  local id = self.params.id
  local repository = Repository:find(id)

  if not repository then
    return self:json({ error = "Not found" }, 404)
  end

  local file_path = self.params.path or ""
  local ref = self.params.ref or GitHelper.get_default_branch(repository.name) or "HEAD"

  local content = GitHelper.get_file_content(repository.name, ref, file_path)
  if not content then
    return self:json({ error = "File not found" }, 404)
  end

  -- Determine content type based on extension
  local file_name = file_path:match("([^/]+)$") or file_path
  local extension = file_name:match("%.([^%.]+)$") or ""

  local content_types = {
    html = "text/html",
    css = "text/css",
    js = "application/javascript",
    json = "application/json",
    xml = "application/xml",
    svg = "image/svg+xml",
    png = "image/png",
    jpg = "image/jpeg",
    jpeg = "image/jpeg",
    gif = "image/gif",
    webp = "image/webp",
    ico = "image/x-icon",
    pdf = "application/pdf"
  }

  local content_type = content_types[extension:lower()] or "text/plain; charset=utf-8"

  self.layout = false
  self:set_header("Content-Type", content_type)
  self:set_header("Content-Disposition", "inline; filename=\"" .. file_name .. "\"")
  self.response.body = content
end

-- Collaborators management page (HTMX partial)
function RepositoriesController:collaborators()
  local current_user = get_current_user()
  local repository = Repository:find(self.params.id)

  if not repository then
    return self:html('<div class="p-4 text-center text-text-dim">Repository not found</div>')
  end

  -- Check ownership
  if repository.owner_id ~= current_user._key then
    return self:html('<div class="p-4 text-center text-red-400">Only the owner can manage collaborators</div>')
  end

  local collaborators = repository:collaborators_with_info()

  self.layout = false
  self:render("repositories/_collaborators", {
    repository = repository,
    collaborators = collaborators
  })
end

-- Search users for adding as collaborators
function RepositoriesController:collaborators_search()
  local current_user = get_current_user()
  local repository = Repository:find(self.params.id)

  if not repository or repository.owner_id ~= current_user._key then
    return self:json({ users = {} })
  end

  local query = self.params.q or ""
  -- If query is empty, list all users (limit 20)
  -- if query == "" then
  --   return self:json({ users = {} })
  -- end

  -- Get current collaborators to exclude
  local collaborators = repository.collaborators or repository.data.collaborators or {}
  local owner_id = repository.owner_id or repository.data.owner_id

  -- Search users by name or email
  -- DEBUG: Print info about the search
  print("DEBUG: User Search - Query: '" .. tostring(query) .. "', Owner ID: " .. tostring(owner_id))
  
  -- Check total user count for debugging
  local count_result = Sdb:Sdbql("RETURN LENGTH(users)")
  print("DEBUG: Total users in DB: " .. tostring(count_result and count_result.result and count_result.result[1] or "error"))

  local result
  if query == "" then
     result = Sdb:Sdbql([[
      FOR u IN users
      FILTER u._key != @owner_id
      RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }
    ]], { owner_id = owner_id })
  else
    result = Sdb:Sdbql([[
      FOR u IN users
      FILTER u._key != @owner_id
      FILTER LOWER(u.email) LIKE CONCAT('%', LOWER(@query), '%')
          OR LOWER(u.firstname) LIKE CONCAT('%', LOWER(@query), '%')
          OR LOWER(u.lastname) LIKE CONCAT('%', LOWER(@query), '%')
      LIMIT 10
      RETURN { _key: u._key, firstname: u.firstname, lastname: u.lastname, email: u.email }
    ]], { query = query, owner_id = owner_id })
  end

  local users = result and result.result or {}

  -- Filter out existing collaborators
  local filtered = {}
  for _, user in ipairs(users) do
    local is_collaborator = false
    for _, collab_key in ipairs(collaborators) do
      -- Handle both string keys and object representations just in case
      local key_to_check = type(collab_key) == "table" and collab_key.user_key or collab_key
      
      if tostring(key_to_check) == tostring(user._key) then
        is_collaborator = true
        break
      end
    end
    if not is_collaborator then
      table.insert(filtered, user)
    end
  end

  self.layout = false
  self:render("repositories/_search_results", { 
    users = filtered, 
    repository = repository,
    query = query 
  })
end

-- Add collaborator
function RepositoriesController:add_collaborator()
  local current_user = get_current_user()
  local repository = Repository:find(self.params.id)

  if not repository then
    return self:json({ error = "Repository not found" }, 404)
  end

  if repository.owner_id ~= current_user._key then
    return self:json({ error = "Only the owner can add collaborators" }, 403)
  end

  local user_key = self.params.user_key
  if not user_key or user_key == "" then
    return self:json({ error = "User key required" }, 400)
  end

  repository:add_collaborator(user_key)

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "collaboratorsChanged")
    return self:html("")
  end

  self:json({ success = true })
end

-- Remove collaborator
function RepositoriesController:remove_collaborator()
  local current_user = get_current_user()
  local repository = Repository:find(self.params.id)

  if not repository then
    return self:json({ error = "Repository not found" }, 404)
  end

  if repository.owner_id ~= current_user._key then
    return self:json({ error = "Only the owner can remove collaborators" }, 403)
  end

  local user_key = self.params.user_key
  repository:remove_collaborator(user_key)

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "collaboratorsChanged")
    return self:html("")
  end

  self:json({ success = true })
end

return RepositoriesController
