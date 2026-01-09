local Controller = require("controller")
local GitHttpController = Controller:extend()
local GitHelper = require("helpers.git_helper")

-- Disable layout for git responses
function GitHttpController:before_action()
  self.layout = false
end

-- GET /git/:repo_path/info/refs?service=...
function GitHttpController:info_refs()
  local repo_path = self.params.repo_path
  local service = self.params.service or GetParam("service")

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

-- POST /git/:repo_path/git-upload-pack
function GitHttpController:upload_pack()
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

-- POST /git/:repo_path/git-receive-pack
function GitHttpController:receive_pack()
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

  for k, v in pairs(response.headers or {}) do
    self:set_header(k, v)
  end

  self.response.body = response.body
end

return GitHttpController
