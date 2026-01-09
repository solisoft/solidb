local Model = require("model")
local GitHelper = require("helpers.git_helper")

local Repository = Model.create("repositories", {
  permitted_fields = { "name", "description", "is_public" },
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

return Repository
