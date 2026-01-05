-- User Model Example
-- app/models/user.lua

local Model = require("model")

local User = Model.create("users", {
  -- Permitted fields for mass assignment protection
  permitted_fields = { "email", "username", "password" },

  -- Validations
  validations = {
    email = {
      presence = true,
      format = { re = "^[^@]+@[^@]+\\.[^@]+$", message = "must be a valid email" }
    },
    username = {
      presence = true,
      length = { between = {3, 50} }
    }
  },

  -- Callbacks
  before_create = {},
  after_create = {},
  before_update = {},
  after_update = {}
})

-- Custom methods can be added here
function User.find_by_email(email)
  return User.find_by({ email = email })
end

function User.find_by_username(username)
  return User.find_by({ username = username })
end

return User
