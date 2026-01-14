local GitConfig = require("git")

local M = {}

-- Lazy load GitSync to avoid breaking git operations if sync module has issues
local _git_sync = nil
local function get_git_sync()
  if _git_sync == nil then
    local ok, mod = pcall(require, "helpers.git_sync")
    if ok then
      _git_sync = mod
    else
      print("Warning: Could not load git_sync: " .. tostring(mod))
      _git_sync = false
    end
  end
  return _git_sync
end

-- Cache for resolved absolute path
local _resolved_repos_path = nil

-- Get the absolute repos path (resolves relative path once)
local function get_absolute_repos_path()
  if _resolved_repos_path then
    return _resolved_repos_path
  end

  local path = GitConfig.repos_path

  -- If already absolute, use as-is
  if path:sub(1, 1) == "/" then
    _resolved_repos_path = path
    return path
  end

  -- Resolve relative path from current working directory
  local handle = io.popen("pwd")
  if handle then
    local cwd = handle:read("*l")
    handle:close()
    if cwd then
      _resolved_repos_path = cwd .. "/" .. path
      return _resolved_repos_path
    end
  end

  -- Fallback to relative path
  _resolved_repos_path = path
  return path
end

-- Helper to check if file/dir exists
local function fs_exists(path)
  local cmd = string.format("test -e '%s'", path)
  return os.execute(cmd)
end

-- Helper to make directory
local function fs_mkdir(path)
  local cmd = string.format("mkdir -p '%s'", path)
  return os.execute(cmd)
end

-- Helper to run a command and capture output (workaround for io.popen issues in Redbean)
local function run_command(cmd)
  local repos_path = get_absolute_repos_path()
  local tmp_dir = repos_path .. "/../tmp"
  fs_mkdir(tmp_dir)
  local output_file = tmp_dir .. "/cmd_output_" .. os.time() .. "_" .. math.random(100000)

  local full_cmd = cmd .. " > '" .. output_file .. "' 2>&1"
  os.execute(full_cmd)

  local handle = io.open(output_file, "r")
  local output = ""
  if handle then
    output = handle:read("*a")
    handle:close()
  end

  os.remove(output_file)
  return output
end

-- Ensure the repos directory exists
function M.ensure_repos_dir()
  local path = get_absolute_repos_path()
  if not fs_exists(path) then
    fs_mkdir(path)
  end
end

-- Get absolute path for a repo
function M.get_repo_path(name)
  -- Sanitize name to prevent directory traversal
  name = name:gsub("%.%.", ""):gsub("/", "")
  if name:sub(-4) ~= ".git" then
    name = name .. ".git"
  end
  return get_absolute_repos_path() .. "/" .. name
end

-- Initialize a new bare repository
function M.init_repo(name)
  M.ensure_repos_dir()
  local path = M.get_repo_path(name)

  -- Check if already exists
  if fs_exists(path) then
    return false, "Repository already exists"
  end

  local cmd = string.format("%s init --bare %s", GitConfig.git_bin, path)
  local success = os.execute(cmd)

  if success then
    -- Create git-daemon-export-ok file to allow http access
    local export_file = path .. "/git-daemon-export-ok"
    local f = io.open(export_file, "w")
    if f then f:close() end

    -- Enable HTTP push (receive-pack)
    os.execute(string.format("%s -C %s config http.receivepack true", GitConfig.git_bin, path))
    os.execute(string.format("%s -C %s config receive.denyCurrentBranch ignore", GitConfig.git_bin, path))

    -- Sync to blob storage for replication (non-blocking, errors logged)
    local sync = get_git_sync()
    if sync then
      pcall(function() sync.push(name) end)
    end

    return true
  else
    return false, "Failed to initialize repository"
  end
end

-- Check if repo exists
function M.repo_exists(name)
  local path = M.get_repo_path(name)
  -- Check for POST-RECEIVE hook or similar to ensure it matches what we expect? 
  -- Or just directory check.
  -- test -d checks for directory
  local cmd = string.format("test -d '%s'", path)
  return os.execute(cmd)
end

-- Get commits for a repo
-- Returns list of { hash, author, date, message }
function M.get_commits(name, branch, limit)
  if not M.repo_exists(name) then return {} end

  local path = M.get_repo_path(name)
  branch = branch or "HEAD"
  limit = limit or 20

  -- Format: hash|author|timestamp|message
  local cmd = string.format(
    "%s --git-dir=%s log %s -n %d --pretty=format:'%%H|%%an|%%at|%%s'",
    GitConfig.git_bin, path, branch, limit
  )

  local output = run_command(cmd)
  local commits = {}

  for line in output:gmatch("[^\n]+") do
    local hash, author, timestamp, message = line:match("^(.-)|(.-)|(.-)|(.*)$")
    if hash then
      table.insert(commits, {
        hash = hash,
        author = author,
        date = os.date("%Y-%m-%d %H:%M:%S", tonumber(timestamp)),
        message = message
      })
    end
  end

  return commits
end

-- Get commits between two refs (e.g. for MR)
function M.get_commits_between(name, from, to)
  if not M.repo_exists(name) then return {} end
  
  local path = M.get_repo_path(name)
  local cmd = string.format(
    "%s --git-dir=%s log %s..%s --pretty=format:'%%H|%%an|%%at|%%s'",
    GitConfig.git_bin, path, from, to
  )
  
  local output = run_command(cmd)
  local commits = {}
  
  for line in output:gmatch("[^\n]+") do
    local hash, author, timestamp, message = line:match("^(.-)|(.-)|(.-)|(.*)$")
    if hash then
      table.insert(commits, {
        hash = hash,
        author = author,
        date = os.date("%Y-%m-%d %H:%M:%S", tonumber(timestamp)),
        message = message
      })
    end
  end
  
  return commits
end

-- Get branches
function M.get_branches(name)
  if not M.repo_exists(name) then return {} end
  local path = M.get_repo_path(name)

  local cmd = string.format("%s --git-dir=%s branch --list", GitConfig.git_bin, path)
  local output = run_command(cmd)

  local branches = {}
  for line in output:gmatch("[^\n]+") do
    local branch = line:match("^[* ]%s*(.+)$")
    if branch then
      table.insert(branches, branch)
    end
  end

  return branches
end

-- Get diff between two commits/branches
function M.get_diff(name, from, to)
  if not M.repo_exists(name) then return "" end
  local path = M.get_repo_path(name)

  local cmd = string.format("%s --git-dir=%s diff %s..%s", GitConfig.git_bin, path, from, to)
  return run_command(cmd)
end

-- Merge source branch into target branch
-- Returns success (bool), error message (string or nil)
function M.merge_branches(name, source_branch, target_branch)
  if not M.repo_exists(name) then
    return false, "Repository does not exist"
  end

  local path = M.get_repo_path(name)

  -- For bare repos, we need to use a worktree or update refs directly
  -- Using git merge with a temporary worktree approach
  local tmp_worktree = os.tmpname() .. "_worktree"
  os.execute(string.format("rm -f '%s'", tmp_worktree))

  -- Create a temporary worktree
  local cmd = string.format("%s --git-dir=%s worktree add '%s' %s 2>&1",
    GitConfig.git_bin, path, tmp_worktree, target_branch)
  local handle = io.popen(cmd)
  local output = handle:read("*a")
  handle:close()

  if not fs_exists(tmp_worktree) then
    return false, "Failed to create worktree: " .. output
  end

  -- Perform the merge in the worktree
  cmd = string.format("cd '%s' && %s merge %s --no-edit 2>&1",
    tmp_worktree, GitConfig.git_bin, source_branch)
  handle = io.popen(cmd)
  output = handle:read("*a")
  local success = handle:close()

  -- Clean up worktree
  os.execute(string.format("rm -rf '%s'", tmp_worktree))
  os.execute(string.format("%s --git-dir=%s worktree prune 2>/dev/null", GitConfig.git_bin, path))

  if success then
    return true, nil
  else
    return false, "Merge failed: " .. output
  end
end

-- Check if branches can be merged (no conflicts)
function M.can_merge(name, source_branch, target_branch)
  if not M.repo_exists(name) then return false, "Repository does not exist" end

  local path = M.get_repo_path(name)

  -- Check if merge would be fast-forward or if there are conflicts
  local cmd = string.format("%s --git-dir=%s merge-base %s %s 2>&1",
    GitConfig.git_bin, path, target_branch, source_branch)
  local handle = io.popen(cmd)
  local merge_base = handle:read("*l")
  handle:close()

  if not merge_base or merge_base == "" then
    return false, "Cannot determine merge base"
  end

  -- Get target branch commit
  cmd = string.format("%s --git-dir=%s rev-parse %s 2>&1", GitConfig.git_bin, path, target_branch)
  handle = io.popen(cmd)
  local target_commit = handle:read("*l")
  handle:close()

  -- If merge base equals target, it's a fast-forward
  if merge_base == target_commit then
    return true, "fast-forward"
  end

  return true, "merge"
end

-- Execute smart HTTP backend
-- This is a wrapper around git-http-backend
-- Uses os.execute with file redirection since io.popen doesn't work in Redbean
function M.handle_smart_http(path_info, method, query_string, body_content, content_type)
  local repo_name = path_info:match("^/([^/]+)")
  if not repo_name then return nil, 404 end

  if not M.repo_exists(repo_name) then return nil, 404 end

  local repos_path = get_absolute_repos_path()

  -- Create temp file for output (use www/tmp for better permissions)
  local tmp_dir = repos_path .. "/../tmp"
  fs_mkdir(tmp_dir)
  local output_file = tmp_dir .. "/git_output_" .. os.time() .. "_" .. math.random(10000)
  local body_file = nil

  -- Build shell script content for proper environment variable handling
  local script_file = tmp_dir .. "/git_script_" .. os.time() .. "_" .. math.random(10000)
  local script = io.open(script_file, "w")
  script:write("#!/bin/sh\n")
  script:write(string.format("export GIT_PROJECT_ROOT='%s'\n", repos_path))
  script:write("export GIT_HTTP_EXPORT_ALL=1\n")
  script:write(string.format("export PATH_INFO='%s'\n", path_info))
  script:write(string.format("export REQUEST_METHOD='%s'\n", method))
  script:write(string.format("export QUERY_STRING='%s'\n", query_string or ""))
  script:write(string.format("export CONTENT_TYPE='%s'\n", content_type or ""))

  local cmd
  if method == "POST" and body_content and #body_content > 0 then
    -- Write body to temp file
    body_file = tmp_dir .. "/git_body_" .. os.time() .. "_" .. math.random(10000)
    local f = io.open(body_file, "wb")
    f:write(body_content)
    f:close()
    script:write(string.format("export CONTENT_LENGTH=%d\n", #body_content))
    script:write(string.format("cat '%s' | %s http-backend\n", body_file, GitConfig.git_bin))
  else
    script:write(string.format("%s http-backend\n", GitConfig.git_bin))
  end
  script:close()

  cmd = string.format("sh '%s' > '%s' 2>&1", script_file, output_file)

  os.execute(cmd)

  -- Read output from file
  local output_handle = io.open(output_file, "rb")
  local output = ""
  if output_handle then
    output = output_handle:read("*a")
    output_handle:close()
  end

  -- Cleanup temp files
  os.remove(script_file)
  os.remove(output_file)
  if body_file then os.remove(body_file) end

  -- Parse output for headers and body
  -- git-http-backend output contains HTTP headers then blank line then body
  local header_end = output:find("\r\n\r\n") or output:find("\n\n")
  if not header_end then
    return { body = output }, 200
  end

  local raw_headers = output:sub(1, header_end - 1)
  local body = output:sub(header_end + (output:find("\r\n\r\n") and 4 or 2))

  local headers = {}
  for line in raw_headers:gmatch("[^\r\n]+") do
    local k, v = line:match("^(.-):%s*(.*)$")
    if k then headers[k] = v end
  end

  -- Remove Status header if present (CGI)
  local status = 200
  if headers["Status"] then
    status = tonumber(headers["Status"]:match("^(%d+)"))
    headers["Status"] = nil
  end

  return {
    headers = headers,
    body = body
  }, status
end

-- Get default branch (usually main or master)
function M.get_default_branch(name)
  if not M.repo_exists(name) then return nil end
  local path = M.get_repo_path(name)

  -- Try to get HEAD reference
  local cmd = string.format("%s --git-dir=%s symbolic-ref --short HEAD", GitConfig.git_bin, path)
  local branch = run_command(cmd):gsub("%s+$", "")

  if branch and branch ~= "" and not branch:match("^fatal") then
    return branch
  end

  -- Fallback: check if main or master exists
  local branches = M.get_branches(name)
  for _, b in ipairs(branches) do
    if b == "main" or b == "master" then
      return b
    end
  end

  return branches[1]
end

-- List files and directories at a given path (tree listing)
-- Returns { type = "blob"|"tree", name = "filename", path = "full/path", commit = {...} }
function M.get_tree(name, ref, tree_path, include_commits)
  if not M.repo_exists(name) then return {} end
  local path = M.get_repo_path(name)
  ref = ref or "HEAD"
  tree_path = tree_path or ""

  -- Build the tree-ish reference
  local tree_ref = ref
  if tree_path and tree_path ~= "" then
    tree_ref = ref .. ":" .. tree_path
  end

  -- Use ls-tree to list contents
  local cmd = string.format("%s --git-dir=%s ls-tree %s",
    GitConfig.git_bin, path, tree_ref)
  local output = run_command(cmd)

  local entries = {}
  for line in output:gmatch("[^\n]+") do
    -- Format: mode type hash\tname
    local mode, obj_type, hash, entry_name = line:match("^(%d+)%s+(%w+)%s+(%x+)%s+(.+)$")
    if entry_name then
      local full_path = tree_path == "" and entry_name or (tree_path .. "/" .. entry_name)
      local entry = {
        mode = mode,
        type = obj_type,  -- "blob" for files, "tree" for directories
        hash = hash,
        name = entry_name,
        path = full_path
      }

      -- Get last commit info for this entry
      if include_commits then
        entry.commit = M.get_last_commit(name, ref, full_path)
      end

      table.insert(entries, entry)
    end
  end

  -- Sort: directories first, then files, alphabetically within each
  table.sort(entries, function(a, b)
    if a.type ~= b.type then
      return a.type == "tree"  -- trees (directories) come first
    end
    return a.name:lower() < b.name:lower()
  end)

  return entries
end

-- Get file content from repo
function M.get_file_content(name, ref, file_path)
  if not M.repo_exists(name) then return nil end
  local path = M.get_repo_path(name)
  ref = ref or "HEAD"

  local cmd = string.format("%s --git-dir=%s show %s:%s",
    GitConfig.git_bin, path, ref, file_path)
  return run_command(cmd)
end

-- Get file size
function M.get_file_size(name, ref, file_path)
  if not M.repo_exists(name) then return 0 end
  local path = M.get_repo_path(name)
  ref = ref or "HEAD"

  local cmd = string.format("%s --git-dir=%s cat-file -s %s:%s",
    GitConfig.git_bin, path, ref, file_path)
  local output = run_command(cmd):gsub("%s+$", "")

  return tonumber(output) or 0
end

-- Check if path is a file or directory
function M.get_path_type(name, ref, check_path)
  if not M.repo_exists(name) then return nil end
  local path = M.get_repo_path(name)
  ref = ref or "HEAD"

  local cmd = string.format("%s --git-dir=%s cat-file -t %s:%s",
    GitConfig.git_bin, path, ref, check_path)
  local obj_type = run_command(cmd):gsub("%s+$", "")

  if obj_type == "" or obj_type:match("^fatal") then
    return nil
  end

  return obj_type  -- "blob" for files, "tree" for directories
end

-- Get last commit for a file/path
function M.get_last_commit(name, ref, file_path)
  if not M.repo_exists(name) then return nil end
  local path = M.get_repo_path(name)
  ref = ref or "HEAD"

  local path_arg = ""
  if file_path and file_path ~= "" then
    path_arg = " -- '" .. file_path .. "'"
  end

  local cmd = string.format("%s --git-dir=%s log -1 --pretty=format:'%%H|%%an|%%at|%%s' %s%s",
    GitConfig.git_bin, path, ref, path_arg)
  local line = run_command(cmd):gsub("%s+$", "")

  if not line or line == "" then return nil end

  local hash, author, timestamp, message = line:match("^(.-)|(.-)|(.-)|(.*)$")
  if hash then
    return {
      hash = hash,
      author = author,
      date = os.date("%Y-%m-%d %H:%M", tonumber(timestamp)),
      timestamp = tonumber(timestamp),
      message = message
    }
  end

  return nil
end

-- Parse raw diff string into structured table
-- Returns list of files, each with hunks and lines
function M.parse_diff(raw_diff)
  if not raw_diff or raw_diff == "" then return {} end

  local files = {}
  local current_file = nil
  local current_hunk = nil
  local old_ln = 0
  local new_ln = 0

  for line in raw_diff:gmatch("[^\n]+") do
    -- Start of a new file
    local file_a, file_b = line:match("^diff %-%-git a/(.+) b/(.+)$")
    if file_a then
      if current_file then
        if current_hunk then table.insert(current_file.hunks, current_hunk) end
        table.insert(files, current_file)
      end
      
      current_file = {
        file_a = file_a,
        file_b = file_b,
        hunks = {}
      }
      current_hunk = nil
    
    -- Hunk header
    elseif line:match("^@@ %-%d+,?%d* %+%d+,?%d* @@") then
      if current_hunk then table.insert(current_file.hunks, current_hunk) end
      
      -- Extract starting line numbers
      -- @@ -OLD_START,OLD_COUNT +NEW_START,NEW_COUNT @@
      local os, ns = line:match("^@@ %-(%d+),?%d* %+(%d+),?%d* @@")
      old_ln = tonumber(os) or 0
      new_ln = tonumber(ns) or 0
      
      current_hunk = {
        header = line,
        lines = {}
      }
      
    -- Content lines
    elseif current_hunk then
      local char = line:sub(1, 1)
      local parsed_line = { content = line }
      
      if char == "+" and not line:match("^%+%+%+") then
        parsed_line.type = "addition"
        parsed_line.new_ln = new_ln
        new_ln = new_ln + 1
      elseif char == "-" and not line:match("^%-%-%-") then
        parsed_line.type = "deletion"
        parsed_line.old_ln = old_ln
        old_ln = old_ln + 1
      elseif char == " " then
        parsed_line.type = "context"
        parsed_line.old_ln = old_ln
        parsed_line.new_ln = new_ln
        old_ln = old_ln + 1
        new_ln = new_ln + 1
      end
      
      if parsed_line.type then
        table.insert(current_hunk.lines, parsed_line)
      end
    end
  end

  -- Add last file
  if current_file then
    if current_hunk then table.insert(current_file.hunks, current_hunk) end
    table.insert(files, current_file)
  end

  return files
end

return M
