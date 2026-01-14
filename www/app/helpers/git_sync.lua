-- Git Sync Helper
-- Syncs git repository folders to/from _git_storage blob collection

local GitConfig = require("git")
local MultipartData = require("multipart")

local M = {}

-- Collection name for git storage
M.COLLECTION = "_git_storage"

-- Get database config
local function get_db_config()
  local db_name = Sdb._db_config and Sdb._db_config.db_name or "_system"
  local db_url = Sdb._db_config and Sdb._db_config.url or "http://localhost:6745"
  return db_name, db_url
end

-- Get auth headers for API calls
local function get_auth_headers()
  local headers = {}

  -- First try Sdb config token
  if Sdb._db_config and Sdb._db_config.token then
    headers["Authorization"] = "Bearer " .. Sdb._db_config.token
    return headers
  end

  -- Try to get from session cookie (web request context)
  if GetCookie then
    local token = GetCookie("sdb_token")
    if token and token ~= "" then
      headers["Authorization"] = "Bearer " .. token
      return headers
    end
  end

  -- Fall back to Basic Auth with configured credentials
  local db_config = require("config.database")
  if db_config and db_config.solidb then
    local username = db_config.solidb.username
    local password = db_config.solidb.password
    if username and password then
      local credentials = EncodeBase64(username .. ":" .. password)
      headers["Authorization"] = "Basic " .. credentials
      return headers
    end
  end

  return headers
end

-- Helper: Get the absolute repos path
local function get_repos_path()
  local path = GitConfig.repos_path
  if path:sub(1, 1) == "/" then
    return path
  end
  -- Resolve relative path
  local handle = io.popen("pwd")
  if handle then
    local cwd = handle:read("*l")
    handle:close()
    if cwd then
      return cwd .. "/" .. path
    end
  end
  return path
end

-- Helper: Get repo path
local function get_repo_path(repo_name)
  local name = repo_name:gsub("%.%.", ""):gsub("/", "")
  if name:sub(-4) ~= ".git" then
    name = name .. ".git"
  end
  return get_repos_path() .. "/" .. name
end

-- Helper: List files recursively in a directory
local function list_files_recursive(dir, base_path)
  base_path = base_path or ""
  local files = {}

  -- Use find command to list all files
  local cmd = string.format("find '%s' -type f 2>/dev/null", dir)
  local handle = io.popen(cmd)
  if handle then
    for line in handle:lines() do
      local relative = line:sub(#dir + 2) -- Remove dir prefix and leading /
      if relative and relative ~= "" then
        table.insert(files, {
          full_path = line,
          relative_path = relative
        })
      end
    end
    handle:close()
  end

  return files
end

-- Helper: Read file content
local function read_file(path)
  local f = io.open(path, "rb")
  if not f then return nil end
  local content = f:read("*a")
  f:close()
  return content
end

-- Helper: Write file content
local function write_file(path, content)
  local f = io.open(path, "wb")
  if not f then return false end
  f:write(content)
  f:close()
  return true
end

-- Helper: Create directory recursively
local function mkdir_p(path)
  os.execute(string.format("mkdir -p '%s'", path))
end

-- Helper: Get directory from path
local function dirname(path)
  return path:match("(.*/)")
end

-- Upload a single file to blob collection
local function upload_blob(db_name, db_url, blob_name, content)
  -- Generate multipart boundary
  local boundary = "----GitSyncBoundary" .. os.time() .. math.random(100000)

  -- Build multipart body manually
  local body_parts = {
    "--" .. boundary,
    'Content-Disposition: form-data; name="file"; filename="' .. blob_name .. '"',
    "Content-Type: application/octet-stream",
    "",
    content,
    "--" .. boundary .. "--"
  }
  local body = table.concat(body_parts, "\r\n")

  local url = db_url .. "/_api/blob/" .. db_name .. "/" .. M.COLLECTION

  local headers = get_auth_headers()
  headers["Content-Type"] = "multipart/form-data; boundary=" .. boundary

  local ok, status, resp_headers, resp_body = pcall(Fetch, url, {
    method = "POST",
    headers = headers,
    body = body
  })

  if not ok then
    return false, "Fetch error: " .. tostring(status)
  end

  if status == 200 or status == 201 then
    local parse_ok, data = pcall(DecodeJson, resp_body)
    if parse_ok and data and data._key then
      return true, data._key
    end
    return false, "Parse error: " .. tostring(resp_body):sub(1, 100)
  end

  -- Return detailed error
  local err_msg = "HTTP " .. tostring(status)
  if resp_body and resp_body ~= "" then
    err_msg = err_msg .. ": " .. tostring(resp_body):sub(1, 100)
  end
  return false, err_msg
end

-- Download a blob by key
local function download_blob(db_name, db_url, blob_key)
  local url = db_url .. "/_api/blob/" .. db_name .. "/" .. M.COLLECTION .. "/" .. blob_key

  local headers = get_auth_headers()

  local status, resp_headers, resp_body = Fetch(url, {
    method = "GET",
    headers = headers
  })

  if status == 200 then
    return true, resp_body
  end

  return false, resp_body
end

-- Delete a blob by key (uses document endpoint)
local function delete_blob(db_name, db_url, blob_key)
  local url = db_url .. "/_api/database/" .. db_name .. "/document/" .. M.COLLECTION .. "/" .. blob_key

  local headers = get_auth_headers()

  local status, resp_headers, resp_body = Fetch(url, {
    method = "DELETE",
    headers = headers
  })

  return status == 200 or status == 204
end

-- Sync repo folder to blob collection (push)
-- Uploads all files in the repo to blob storage
function M.push(repo_name)
  local db_name, db_url = get_db_config()
  local repo_path = get_repo_path(repo_name)

  -- Check if repo exists
  local test_cmd = string.format("test -d '%s'", repo_path)
  if not os.execute(test_cmd) then
    return false, "Repository folder does not exist: " .. repo_path
  end

  -- First, delete existing blobs for this repo (clean sync)
  M.delete(repo_name)

  -- List all files in the repo
  local files = list_files_recursive(repo_path)
  local prefix = repo_name .. ".git/"

  local uploaded = 0
  local errors = {}

  for _, file in ipairs(files) do
    local content = read_file(file.full_path)
    if content then
      local blob_name = prefix .. file.relative_path
      local ok, result = upload_blob(db_name, db_url, blob_name, content)
      if ok then
        uploaded = uploaded + 1
      else
        table.insert(errors, file.relative_path .. ": " .. tostring(result))
      end
    else
      table.insert(errors, file.relative_path .. ": Could not read file")
    end
  end

  if #errors > 0 then
    return false, "Uploaded " .. uploaded .. " files, " .. #errors .. " errors: " .. table.concat(errors, "; ")
  end

  return true, "Uploaded " .. uploaded .. " files"
end

-- Restore repo folder from blob collection (pull)
-- Downloads all blobs and recreates the folder structure
function M.pull(repo_name)
  local db_name, db_url = get_db_config()
  local repo_path = get_repo_path(repo_name)
  local prefix = repo_name .. ".git/"

  -- Query all blobs for this repo by name prefix
  local query = string.format([[
    FOR doc IN %s
    FILTER doc.name != null AND STARTS_WITH(doc.name, @prefix)
    RETURN { _key: doc._key, name: doc.name }
  ]], M.COLLECTION)

  local result = Sdb:Sdbql(query, { prefix = prefix })

  if not result or not result.result or #result.result == 0 then
    return false, "No blobs found for repository: " .. repo_name
  end

  -- Ensure repos directory exists
  mkdir_p(get_repos_path())

  local downloaded = 0
  local errors = {}

  for _, blob in ipairs(result.result) do
    -- Extract relative path from blob name
    local relative_path = blob.name:sub(#prefix + 1)
    local full_path = repo_path .. "/" .. relative_path

    -- Create directory if needed
    local dir = dirname(full_path)
    if dir then
      mkdir_p(dir)
    end

    -- Download blob content
    local ok, content = download_blob(db_name, db_url, blob._key)
    if ok then
      if write_file(full_path, content) then
        downloaded = downloaded + 1
      else
        table.insert(errors, relative_path .. ": Could not write file")
      end
    else
      table.insert(errors, relative_path .. ": " .. tostring(content))
    end
  end

  if #errors > 0 then
    return false, "Downloaded " .. downloaded .. " files, " .. #errors .. " errors: " .. table.concat(errors, "; ")
  end

  return true, "Downloaded " .. downloaded .. " files"
end

-- Delete all blobs for a repo
function M.delete(repo_name)
  local db_name, db_url = get_db_config()
  local prefix = repo_name .. ".git/"

  -- Query all blobs for this repo
  local query = string.format([[
    FOR doc IN %s
    FILTER doc.name != null AND STARTS_WITH(doc.name, @prefix)
    RETURN doc._key
  ]], M.COLLECTION)

  local result = Sdb:Sdbql(query, { prefix = prefix })

  if not result or not result.result then
    return true, "No blobs to delete"
  end

  local deleted = 0
  for _, key in ipairs(result.result) do
    if delete_blob(db_name, db_url, key) then
      deleted = deleted + 1
    end
  end

  return true, "Deleted " .. deleted .. " blobs"
end

-- Check if repo exists in blob storage
function M.exists(repo_name)
  local prefix = repo_name .. ".git/"

  local query = string.format([[
    FOR doc IN %s
    FILTER doc.name != null AND STARTS_WITH(doc.name, @prefix)
    LIMIT 1
    RETURN 1
  ]], M.COLLECTION)

  local result = Sdb:Sdbql(query, { prefix = prefix })

  return result and result.result and #result.result > 0
end

-- Get sync status for a repo
function M.status(repo_name)
  local prefix = repo_name .. ".git/"

  local query = string.format([[
    FOR doc IN %s
    FILTER doc.name != null AND STARTS_WITH(doc.name, @prefix)
    COLLECT WITH COUNT INTO total
    RETURN total
  ]], M.COLLECTION)

  local result = Sdb:Sdbql(query, { prefix = prefix })
  local blob_count = (result and result.result and result.result[1]) or 0

  -- Count local files
  local repo_path = get_repo_path(repo_name)
  local files = list_files_recursive(repo_path)
  local file_count = #files

  return {
    blob_count = blob_count,
    file_count = file_count,
    synced = blob_count > 0
  }
end

return M
