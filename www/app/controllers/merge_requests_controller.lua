local Controller = require("controller")
local MergeRequestsController = Controller:extend()
local MergeRequest = require("models.merge_request")
local Repository = require("models.repository")
local GitHelper = require("helpers.git_helper")
local AuthHelper = require("helpers.auth_helper")

function MergeRequestsController:before_action()
  self.layout = "talks"
  self.current_user = AuthHelper.require_login(self, "/repositories")
  if not self.current_user then return end
end

-- Retrieve repo helper
-- Retrieve repo helper
function MergeRequestsController:get_repo()
  -- Try to get repo_id from various possible router params for nested resources
  -- Router naive pluralization of 'repositories' results in 'repositorie_id'
  local repo_id = self.params.repo_id or self.params.repository_id or self.params.repositorie_id

  -- If still nil, parameters might be flattened
  if not repo_id and self.params.repo_path then
     -- If we have repo_path (from git routes potentially?), handle it
     -- But here we likely just have ID issues
  end
  if not repo_id then
    -- If we are in a nested route /repositories/:id/..., the direct id might be captured as 'id'
    -- if the router logic flattens it, but usually 'id' is the resource id (MR id).
    -- However, for the INDEX action, there is no MR id, so 'id' might be the repo id.
    if not self.params.id then
       print("ERROR: No repo_id found in params")
       -- return nil will cause 404 below
    elseif self.action_name == "index" or self.action_name == "new_form" or self.action_name == "create" then
       -- For collection actions, 'id' might be the parent id if router doesn't namespace it
       repo_id = self.params.id
    end
  end

  if not repo_id then
    self:render("errors/404", {}, { status = 404 })
    return nil
  end

  local repo = Repository:find(repo_id)
  if not repo then
    self:render("errors/404", {}, { status = 404 })
    return nil
  end
  return repo
end

-- List MRs for a repository
function MergeRequestsController:index()
  local repo = self:get_repo()
  if not repo then return end

  local mrs = MergeRequest.where({ repo_id = repo.id }):all()
  local open_mrs = 0
  for _, mr in ipairs(mrs) do
    if mr.status == "open" then open_mrs = open_mrs + 1 end
  end

  self:render("merge_requests/index", {
    repository = repo,
    merge_requests = mrs,
    open_mrs_count = open_mrs
  })
end

-- New MR form
function MergeRequestsController:new_form()
  local repo = self:get_repo()
  if not repo then return end

  local branches = GitHelper.get_branches(repo.name)

  self:render("merge_requests/new", {
    repository = repo,
    merge_request = MergeRequest:new(),
    branches = branches
  })
end

-- Compare branches
function MergeRequestsController:compare()
  print("DEBUG: Compare action triggered")
  local repo = self:get_repo()
  if not repo then 
    print("DEBUG: Repo not found")
    return 
  end

  local source = self.params.source_branch
  local target = self.params.target_branch
  
  print("DEBUG: Comparing " .. tostring(source) .. " to " .. tostring(target))

  if not source or not target then
    return self:render("merge_requests/_compare_result", { error = "Both branches are required" }, { layout = false })
  end

  if source == target then
    return self:render("merge_requests/_compare_result", { error = "Source and target branches must be different" }, { layout = false })
  end

  local raw_diff = GitHelper.get_diff(repo.name, target, source)
  local diff = GitHelper.parse_diff(raw_diff)
  local commits = GitHelper.get_commits_between(repo.name, target, source)
  
  print("DEBUG: Diff files: " .. (#diff or 0))
  print("DEBUG: Commits count: " .. (#commits or 0))

  self:render("merge_requests/_compare_result", {
    diff = diff,
    commits = commits,
    source_branch = source,
    target_branch = target
  }, { layout = false })
end

-- Create MR
function MergeRequestsController:create()
  local repo = self:get_repo()
  if not repo then return end

  local params = MergeRequest.permit(self.params.merge_request)
  params.repo_id = repo.id
  params.author_id = self.current_user._key
  params.status = "open"
  params.created_at = os.time()

  local mr = MergeRequest:new(params)

  if mr:save() then
    self:redirect("/repositories/" .. repo.id .. "/merge_requests/" .. mr.id)
  else
    local branches = GitHelper.get_branches(repo.name)
    self:render("merge_requests/new", {
      repository = repo,
      merge_request = mr,
      branches = branches
    })
  end
end

-- Show MR
function MergeRequestsController:show()
  local repo = self:get_repo()
  if not repo then return end

  local id = self.params.id
  local mr = MergeRequest:find(id)

  if not mr then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Get diff
  local raw_diff = GitHelper.get_diff(repo.name, mr.target_branch, mr.source_branch)
  local diff = GitHelper.parse_diff(raw_diff)

  -- Get comments and enrich with author names
  local MrComment = require("models.mr_comment")
  local comments = MrComment.where({ mr_id = mr.id }):all()

  -- Enrich comments with author info
  local db = _G.Sdb
  if db then
    for _, comment in ipairs(comments) do
      if comment.author_id then
        local userRes = db:Sdbql("FOR u IN users FILTER u._key == @key RETURN { name: u.name, username: u.username }", { key = comment.author_id })
        if userRes and userRes.result and userRes.result[1] then
          comment.author_name = userRes.result[1].name or userRes.result[1].username
        end
      end
    end
  end

  self:render("merge_requests/show", {
    repository = repo,
    merge_request = mr,
    diff = diff,
    comments = comments,
    current_user = self.current_user
  })
end

-- Add comment
function MergeRequestsController:add_comment()
  local repo = self:get_repo()
  if not repo then return end

  local mr_id = self.params.id
  local content = self.params.content

  local MrComment = require("models.mr_comment")
  local comment = MrComment:new({
    mr_id = mr_id,
    author_id = self.current_user._key,
    content = content,
    created_at = os.time()
  })

  comment:save()

  self:redirect("/repositories/" .. repo.id .. "/merge_requests/" .. mr_id)
end

-- Edit MR form
function MergeRequestsController:edit()
  local repo = self:get_repo()
  if not repo then return end

  local id = self.params.id
  local mr = MergeRequest:find(id)

  if not mr then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership
  if mr.author_id ~= self.current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  local branches = GitHelper.get_branches(repo.name)

  self:render("merge_requests/edit", {
    repository = repo,
    merge_request = mr,
    branches = branches
  })
end

-- Update MR
function MergeRequestsController:update()
  local repo = self:get_repo()
  if not repo then return end

  local id = self.params.id
  local mr = MergeRequest:find(id)

  if not mr then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership
  if mr.author_id ~= self.current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  local params = MergeRequest.permit(self.params.merge_request)

  for k, v in pairs(params) do
    mr[k] = v
  end

  if mr:save() then
    self:redirect("/repositories/" .. repo.id .. "/merge_requests/" .. id)
  else
    local branches = GitHelper.get_branches(repo.name)
    self:render("merge_requests/edit", {
      repository = repo,
      merge_request = mr,
      branches = branches
    })
  end
end

-- Close MR
function MergeRequestsController:close()
  local repo = self:get_repo()
  if not repo then return end

  local id = self.params.id
  local mr = MergeRequest:find(id)

  if not mr then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership or repo owner
  if mr.author_id ~= self.current_user._key and repo.owner_id ~= self.current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  mr.status = "closed"
  mr:save()

  self:redirect("/repositories/" .. repo.id .. "/merge_requests/" .. id)
end

-- Reopen MR
function MergeRequestsController:reopen()
  local repo = self:get_repo()
  if not repo then return end

  local id = self.params.id
  local mr = MergeRequest:find(id)

  if not mr then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership or repo owner
  if mr.author_id ~= self.current_user._key and repo.owner_id ~= self.current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  mr.status = "open"
  mr:save()

  self:redirect("/repositories/" .. repo.id .. "/merge_requests/" .. id)
end

-- Merge MR
function MergeRequestsController:merge()
  local repo = self:get_repo()
  if not repo then return end

  local id = self.params.id
  local mr = MergeRequest:find(id)

  if not mr then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Only repo owner can merge
  if repo.owner_id ~= self.current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  -- Only open MRs can be merged
  if mr.status ~= "open" then
    self:redirect("/repositories/" .. repo.id .. "/merge_requests/" .. id)
    return
  end

  -- Perform the merge
  local success, err = GitHelper.merge_branches(repo.name, mr.source_branch, mr.target_branch)

  if success then
    mr.status = "merged"
    mr.merged_at = os.time()
    mr.merged_by = self.current_user._key
    mr:save()
  else
    -- Store error for display
    mr.merge_error = err
  end

  self:redirect("/repositories/" .. repo.id .. "/merge_requests/" .. id)
end

-- Delete MR
function MergeRequestsController:destroy()
  local repo = self:get_repo()
  if not repo then return end

  local id = self.params.id
  local mr = MergeRequest:find(id)

  if not mr then
    return self:render("errors/404", {}, { status = 404 })
  end

  -- Check ownership
  if mr.author_id ~= self.current_user._key and repo.owner_id ~= self.current_user._key then
    return self:render("errors/403", {}, { status = 403 })
  end

  -- Delete all comments first
  local MrComment = require("models.mr_comment")
  local comments = MrComment.where({ mr_id = id }):all()
  for _, comment in ipairs(comments) do
    comment:destroy()
  end

  mr:destroy()

  self:redirect("/repositories/" .. repo.id .. "/merge_requests")
end

return MergeRequestsController
