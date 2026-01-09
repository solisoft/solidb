local Model = require("model")

local MrComment = Model.create("mr_comments", {
  permitted_fields = { "mr_id", "author_id", "content" },
  validations = {
    mr_id = { presence = true },
    author_id = { presence = true },
    content = { presence = true }
  }
})

return MrComment
