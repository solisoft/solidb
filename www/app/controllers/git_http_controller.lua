local Controller = require("controller")
local GitHttpController = Controller:extend()
local GitHelper = require("helpers.git_helper")
local Repository = require("models.repository")
local argon2 = require("argon2")

-- Disable layout for git responses
function GitHttpController:before_action()
  self.layout = false
end

-- Parse HTTP Basic Auth header
local function parse_basic_auth()
  local auth_header = GetHeader("Authorization")
  if not auth_header then return nil, nil end

  local auth_type, credentials = auth_header:match("^(%w+)%s+(.+)$")
  if auth_type ~= "Basic" then return nil, nil end

  -- Decode base64
  local decoded = DecodeBase64(credentials)
  if not decoded then return nil, nil end

  local username, password = decoded:match("^([^:]+):(.*)$")
  return username, password
end

-- Authenticate user with HTTP Basic Auth
local function authenticate_user()
  local username, password = parse_basic_auth()
  if not username or not password then
    return nil
  end

  -- Find user by email (username in git is typically email)
  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER u.email == @email
    LIMIT 1
    RETURN u
  ]], { email = username })

  local user = result and result.result and result.result[1]
  if not user then return nil end

  -- Verify password
  local valid = argon2.verify(user.password_hash, password)
  if not valid then return nil end

  return user
end

-- Send 401 Unauthorized response
local function send_unauthorized(self)
  self:set_header("WWW-Authenticate", 'Basic realm="Git Repository"')
  self:status(401)
  self.response.body = "Unauthorized"
end

-- Send 403 Forbidden response
local function send_forbidden(self)
  self:status(403)
  self.response.body = "Forbidden: You don't have access to this repository"
end

-- Check repository access
-- Returns: repo, user, error_handler
local function check_repo_access(self, require_push)
  local repo_path = self.params.repo_path
  -- repo_path is like "myrepo.git" - strip .git suffix
  local repo_name = repo_path:gsub("%.git$", "")

  -- Find repository
  local repo = Repository.find_by_name(repo_name)
  if not repo then
    self:status(404)
    self.response.body = "Repository not found"
    return nil, nil, true
  end

  -- Auto-restore from blob storage if folder is missing
  if not GitHelper.repo_exists(repo_name) then
    self:status(404)
    self.response.body = "Repository folder missing - please restore first"
    return nil, nil, true
  end

  -- Check if repo is public (for read operations)
  local is_public = repo.is_public or (repo.data and repo.data.is_public)

  -- For public repos, allow anonymous read (upload-pack/info-refs for clone)
  if is_public and not require_push then
    return repo, nil, false
  end

  -- Authenticate user
  local user = authenticate_user()
  if not user then
    send_unauthorized(self)
    return nil, nil, true
  end

  -- Check access
  if require_push then
    if not repo:user_can_push(user._key) then
      send_forbidden(self)
      return nil, nil, true
    end
  else
    if not repo:user_has_access(user._key) then
      send_forbidden(self)
      return nil, nil, true
    end
  end

  return repo, user, false
end

-- GET /git/:repo_path/info/refs?service=...
function GitHttpController:info_refs()
  local service = self.params.service or GetParam("service")
  local require_push = (service == "git-receive-pack")

  local repo, user, has_error = check_repo_access(self, require_push)
  if has_error then return end

  local repo_path = self.params.repo_path
  local path_info = "/" .. repo_path .. "/info/refs"
  local query_string = "service=" .. (service or "")

  local response, status = GitHelper.handle_smart_http(path_info, "GET", query_string, nil)

  if not response then
    return self:json({ error = "Internal Server Error" }, 500)
  end

  -- Set headers
  for k, v in pairs(response.headers or {}) do
    self:set_header(k, v)
  end

  -- Force no-cache
  self:set_header("Expires", "Fri, 01 Jan 1980 00:00:00 GMT")
  self:set_header("Pragma", "no-cache")
  self:set_header("Cache-Control", "no-cache, max-age=0, must-revalidate")

  self.response.body = response.body
end

-- POST /git/:repo_path/git-upload-pack (clone/fetch - read operation)
function GitHttpController:upload_pack()
  local repo, user, has_error = check_repo_access(self, false) -- read access
  if has_error then return end

  local repo_path = self.params.repo_path
  local path_info = "/" .. repo_path .. "/git-upload-pack"

  -- Get body from Redbean's global function
  local body = GetBody()
  -- Get Content-Type header
  local content_type = GetHeader("Content-Type")

  local response, status = GitHelper.handle_smart_http(path_info, "POST", nil, body, content_type)

  if not response then
    return self:json({ error = "Internal Server Error" }, 500)
  end

  for k, v in pairs(response.headers or {}) do
    self:set_header(k, v)
  end

  self.response.body = response.body
end

-- POST /git/:repo_path/git-receive-pack (push - write operation)
function GitHttpController:receive_pack()
  local repo, user, has_error = check_repo_access(self, true) -- push access required
  if has_error then return end

  local repo_path = self.params.repo_path
  local path_info = "/" .. repo_path .. "/git-receive-pack"

  -- Get body from Redbean's global function
  local body = GetBody()
  -- Get Content-Type header
  local content_type = GetHeader("Content-Type")

  local response, status = GitHelper.handle_smart_http(path_info, "POST", nil, body, content_type)

  if not response then
    return self:json({ error = "Internal Server Error" }, 500)
  end

  -- Sync to blob storage after successful push (non-blocking)
  -- Extract repo name from path (e.g., "myrepo.git" -> "myrepo")
  local repo_name = repo_path:gsub("%.git$", "")
  pcall(function()
    local GitSync = require("helpers.git_sync")
    GitSync.push(repo_name)
  end)

  for k, v in pairs(response.headers or {}) do
    self:set_header(k, v)
  end

  self.response.body = response.body
end

return GitHttpController
