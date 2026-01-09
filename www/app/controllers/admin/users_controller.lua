local Controller = require("controller")
local UsersController = Controller:extend()
local AuthHelper = require("helpers.auth_helper")
local InvitationToken = require("models.invitation_token")

-- Get current user (middleware ensures user is authenticated)
local function get_current_user()
  return AuthHelper.get_current_user()
end

-- Check if current user is admin
local function require_admin(self)
  local current_user = get_current_user()
  if not current_user then
    return nil
  end

  -- Check if user is admin
  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER u._key == @key
    RETURN { is_admin: u.is_admin }
  ]], { key = current_user._key })

  if result and result.result and result.result[1] and result.result[1].is_admin then
    return current_user
  end

  return nil
end

-- List all users
function UsersController:index()
  local current_user = require_admin(self)
  if not current_user then
    return self:redirect("/talks")
  end

  local result = Sdb:Sdbql([[
    FOR u IN users
    SORT u.created_at DESC
    RETURN {
      _key: u._key,
      _id: u._id,
      firstname: u.firstname,
      lastname: u.lastname,
      email: u.email,
      is_admin: u.is_admin,
      status: u.status,
      created_at: u.created_at
    }
  ]])

  local users = (result and result.result) or {}

  -- Get pending invitations count
  local pending_invitations = InvitationToken.pending_count()

  self.layout = "talks"
  self:render("admin/users/index", {
    current_user = current_user,
    users = users,
    pending_invitations = pending_invitations
  })
end

-- Show user details
function UsersController:show()
  local current_user = require_admin(self)
  if not current_user then
    return self:redirect("/talks")
  end

  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER u._key == @key
    RETURN u
  ]], { key = self.params.key })

  local user = result and result.result and result.result[1]
  if not user then
    return self:redirect("/admin/users")
  end

  self.layout = "talks"
  self:render("admin/users/show", {
    current_user = current_user,
    user = user
  })
end

-- Edit user form
function UsersController:edit()
  local current_user = require_admin(self)
  if not current_user then
    return self:redirect("/talks")
  end

  local result = Sdb:Sdbql([[
    FOR u IN users
    FILTER u._key == @key
    RETURN u
  ]], { key = self.params.key })

  local user = result and result.result and result.result[1]
  if not user then
    return self:redirect("/admin/users")
  end

  self.layout = "talks"
  self:render("admin/users/edit", {
    current_user = current_user,
    user = user
  })
end

-- Update user
function UsersController:update()
  local current_user = require_admin(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 403)
  end

  local key = self.params.key

  local updates = {}
  if self.params.firstname then updates.firstname = self.params.firstname end
  if self.params.lastname then updates.lastname = self.params.lastname end
  if self.params.is_admin ~= nil then
    updates.is_admin = self.params.is_admin == "true" or self.params.is_admin == true
  end

  if next(updates) then
    Sdb:Sdbql("UPDATE @key WITH @updates IN users", { key = key, updates = updates })
  end

  if self:is_htmx_request() then
    self:set_header("HX-Redirect", "/admin/users")
    return self:html("")
  end

  return self:redirect("/admin/users")
end

-- Delete user
function UsersController:destroy()
  local current_user = require_admin(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 403)
  end

  local key = self.params.key

  -- Don't allow deleting yourself
  if key == current_user._key then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400">Cannot delete your own account</div>')
    end
    return self:redirect("/admin/users")
  end

  Sdb:Sdbql("REMOVE @key IN users", { key = key })

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "userDeleted")
    return self:html("")
  end

  return self:redirect("/admin/users")
end

-- List invitations
function UsersController:invitations()
  local current_user = require_admin(self)
  if not current_user then
    return self:redirect("/talks")
  end

  local invitations = InvitationToken.all_with_status()

  -- Enrich with creator info
  for _, inv in ipairs(invitations) do
    if inv.created_by then
      local creator = Sdb:Sdbql([[
        FOR u IN users
        FILTER u._key == @key
        RETURN { firstname: u.firstname, lastname: u.lastname }
      ]], { key = inv.created_by })
      if creator and creator.result and creator.result[1] then
        inv.creator = creator.result[1]
      end
    end
  end

  self.layout = "talks"
  self:render("admin/users/invitations", {
    current_user = current_user,
    invitations = invitations
  })
end

-- Create invitation
function UsersController:create_invitation()
  local current_user = require_admin(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 403)
  end

  local email = self.params.email
  if email == "" then email = nil end

  local expires_in_days = tonumber(self.params.expires_in_days) or 7

  local invitation = InvitationToken.create_invitation(current_user._key, email, expires_in_days)

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "invitationCreated")
    -- Return the new invitation details for display
    local token = invitation.token or (invitation.data and invitation.data.token) or (invitation.attributes and invitation.attributes.token)
    if not token then
      return self:html([[
        <div class="bg-success/10 border border-success/20 rounded-lg p-3 mb-3">
          <p class="text-success text-sm font-medium"><i class="fas fa-check mr-2"></i>Invitation created! Refresh to see it.</p>
        </div>
      ]])
    end
    local signup_url = "/auth/signup?token=" .. token
    return self:html([[
      <div class="bg-success/10 border border-success/20 rounded-lg p-3 mb-3">
        <p class="text-success text-sm font-medium mb-2"><i class="fas fa-check mr-2"></i>Invitation created!</p>
        <p class="text-[11px] text-text-dim mb-2">Share this link:</p>
        <div class="flex items-center gap-2">
          <code class="flex-1 bg-black/30 rounded px-2 py-1.5 text-xs text-white font-mono truncate">]] .. signup_url .. [[</code>
          <button onclick="navigator.clipboard.writeText(window.location.origin + ']] .. signup_url .. [['); this.innerHTML='<i class=\\'fas fa-check\\'></i>'"
                  class="px-2 py-1.5 bg-white/10 hover:bg-white/20 text-white text-xs rounded transition-colors">
            <i class="fas fa-copy"></i>
          </button>
        </div>
      </div>
    ]])
  end

  return self:redirect("/admin/invitations")
end

-- Delete invitation
function UsersController:delete_invitation()
  local current_user = require_admin(self)
  if not current_user then
    return self:json({ error = "Unauthorized" }, 403)
  end

  Sdb:Sdbql("REMOVE @key IN invitation_tokens", { key = self.params.key })

  if self:is_htmx_request() then
    self:set_header("HX-Trigger", "invitationDeleted")
    return self:html("")
  end

  return self:redirect("/admin/invitations")
end

return UsersController
