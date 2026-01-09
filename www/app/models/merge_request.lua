local Model = require("model")

local MergeRequest = Model.create("merge_requests", {
  permitted_fields = { "repo_id", "title", "description", "source_branch", "target_branch", "status" },
  validations = {
    repo_id = { presence = true },
    title = { presence = true },
    source_branch = { presence = true },
    target_branch = { presence = true }
  }
})

return MergeRequest
