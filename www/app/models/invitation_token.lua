local Model = require("model")

local InvitationToken = Model.create("invitation_tokens", {
  permitted_fields = { "email", "token", "expires_at", "used_at", "created_by" },
  validations = {
    token = { presence = true },
    created_by = { presence = true }
  }
})

-- Generate a random token
function InvitationToken.generate_token()
  local chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
  local token = ""
  for i = 1, 32 do
    local idx = math.random(1, #chars)
    token = token .. chars:sub(idx, idx)
  end
  return token
end

-- Create a new invitation token
function InvitationToken.create_invitation(created_by, email, expires_in_days)
  expires_in_days = expires_in_days or 7
  local token = InvitationToken.generate_token()
  local expires_at = os.time() + (expires_in_days * 24 * 60 * 60)

  local invitation = InvitationToken:create({
    token = token,
    email = email or nil,
    expires_at = expires_at,
    used_at = nil,
    created_by = created_by,
    created_at = os.time()
  })

  -- Ensure token is accessible on the returned object
  if invitation then
    invitation.token = token
  end

  return invitation
end

-- Find a valid token (not expired, not used)
function InvitationToken.find_valid(token)
  local result = Sdb:Sdbql([[
    FOR t IN invitation_tokens
    FILTER t.token == @token
    FILTER t.used_at == null
    FILTER t.expires_at > @now
    LIMIT 1
    RETURN t
  ]], { token = token, now = os.time() })

  if result and result.result and result.result[1] then
    local inv = InvitationToken:new()
    inv.data = result.result[1]
    inv._key = result.result[1]._key
    inv._id = result.result[1]._id
    for k, v in pairs(result.result[1]) do
      inv[k] = v
    end
    return inv
  end
  return nil
end

-- Mark token as used
function InvitationToken:mark_used()
  self:update({ used_at = os.time() })
end

-- Check if token is valid for a specific email (if email was specified)
function InvitationToken:valid_for_email(email)
  local token_email = self.email or (self.data and self.data.email)
  if not token_email then
    return true -- No email restriction
  end
  return token_email:lower() == email:lower()
end

-- Get all tokens (for admin)
function InvitationToken.all_with_status()
  local result = Sdb:Sdbql([[
    FOR t IN invitation_tokens
    SORT t.created_at DESC
    LIMIT 100
    RETURN t
  ]])
  return (result and result.result) or {}
end

-- Count pending (unused, not expired) tokens
function InvitationToken.pending_count()
  local result = Sdb:Sdbql([[
    RETURN LENGTH(
      FOR t IN invitation_tokens
      FILTER t.used_at == null AND t.expires_at > @now
      RETURN 1
    )
  ]], { now = os.time() })
  return (result and result.result and result.result[1]) or 0
end

return InvitationToken
