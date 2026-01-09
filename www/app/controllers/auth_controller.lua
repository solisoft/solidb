local Controller = require("controller")
local AuthController = Controller:extend()
local argon2 = require("argon2")
local InvitationToken = require("models.invitation_token")

local function get_db()
  return _G.Sdb
end

-- Check if any users exist in database
local function has_users()
  local db = get_db()
  local result = db:Sdbql("RETURN LENGTH(users)")
  return result and result.result and result.result[1] and result.result[1] > 0
end

-- Login Form (GET /auth/login)
function AuthController:login()
  self.layout = "auth"
  local redirect_to = self.params.redirect or "/talks"
  local flash = GetFlash()
  self:render("auth/login", {
    redirect_to = redirect_to,
    error = flash.error
  })
end

-- Login Action (POST /auth/login)
function AuthController:do_login()
  local email = self.params.email
  local password = self.params.password
  local redirect_to = self.params.redirect or "/talks"
  local db = get_db()

  if not email or not password or email == "" or password == "" then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400 text-sm mt-2">Email and password are required</div>')
    end
    self:set_flash("error", "Email and password are required")
    return self:redirect("/auth/login?redirect=" .. redirect_to)
  end

  -- Find user by email
  local usersRes = db:Sdbql("FOR u IN users FILTER u.email == @email LIMIT 1 RETURN u", { email = email })
  local user = usersRes and usersRes.result and usersRes.result[1]

  if not user then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400 text-sm mt-2">Invalid email or password</div>')
    end
    self:set_flash("error", "Invalid email or password")
    return self:redirect("/auth/login?redirect=" .. redirect_to)
  end

  -- Verify password
  local valid = argon2.verify(user.password_hash, password)
  if not valid then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400 text-sm mt-2">Invalid email or password</div>')
    end
    self:set_flash("error", "Invalid email or password")
    return self:redirect("/auth/login?redirect=" .. redirect_to)
  end

  -- Set session (24 hours = 1440 minutes)
  self:set_session({
    user_id = user._id,
    user_data = {
      _id = user._id,
      _key = user._key,
      email = user.email,
      firstname = user.firstname,
      lastname = user.lastname,
      is_admin = user.is_admin or false,
      channel_last_seen = user.channel_last_seen or {}
    }
  }, 24 * 60)

  -- For HTMX, send HX-Redirect header with 200 status
  if self:is_htmx_request() then
    self:set_header("HX-Redirect", redirect_to)
    return self:html("")
  end

  return self:redirect(redirect_to)
end

-- Signup Form (GET /auth/signup)
function AuthController:signup()
  self.layout = "auth"
  local redirect_to = self.params.redirect or "/talks"
  local token = self.params.token or ""
  local flash = GetFlash()

  -- Check if this is first user (no token required)
  local is_first_user = not has_users()

  -- If not first user and no token provided, show error
  if not is_first_user and (not token or token == "") then
    self:render("auth/signup", {
      redirect_to = redirect_to,
      error = "An invitation token is required to sign up",
      token_required = true,
      is_first_user = false
    })
    return
  end

  -- If token provided, validate it
  local invitation = nil
  if token and token ~= "" then
    invitation = InvitationToken.find_valid(token)
    if not invitation then
      self:render("auth/signup", {
        redirect_to = redirect_to,
        error = "Invalid or expired invitation token",
        token_required = true,
        is_first_user = false
      })
      return
    end
  end

  self:render("auth/signup", {
    redirect_to = redirect_to,
    error = flash.error,
    token = token,
    token_required = not is_first_user,
    is_first_user = is_first_user,
    invitation_email = invitation and (invitation.email or invitation.data.email) or nil
  })
end

-- Signup Action (POST /auth/signup)
function AuthController:do_signup()
  local firstname = self.params.firstname
  local lastname = self.params.lastname
  local email = self.params.email
  local password = self.params.password
  local token = self.params.token or ""
  local redirect_to = self.params.redirect or "/talks"
  local db = get_db()

  -- Check if this is first user
  local is_first_user = not has_users()

  -- Token validation (unless first user)
  local invitation = nil
  if not is_first_user then
    if not token or token == "" then
      if self:is_htmx_request() then
        return self:html('<div class="text-red-400 text-sm mt-2">An invitation token is required</div>')
      end
      self:set_flash("error", "An invitation token is required")
      return self:redirect("/auth/signup?redirect=" .. redirect_to)
    end

    invitation = InvitationToken.find_valid(token)
    if not invitation then
      if self:is_htmx_request() then
        return self:html('<div class="text-red-400 text-sm mt-2">Invalid or expired invitation token</div>')
      end
      self:set_flash("error", "Invalid or expired invitation token")
      return self:redirect("/auth/signup?redirect=" .. redirect_to)
    end

    -- Check if token is for specific email
    if not invitation:valid_for_email(email) then
      if self:is_htmx_request() then
        return self:html('<div class="text-red-400 text-sm mt-2">This invitation is for a different email address</div>')
      end
      self:set_flash("error", "This invitation is for a different email address")
      return self:redirect("/auth/signup?token=" .. token .. "&redirect=" .. redirect_to)
    end
  end

  -- Validation
  if not firstname or not lastname or not email or not password or
     firstname == "" or lastname == "" or email == "" or password == "" then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400 text-sm mt-2">All fields are required</div>')
    end
    self:set_flash("error", "All fields are required")
    return self:redirect("/auth/signup?token=" .. token .. "&redirect=" .. redirect_to)
  end

  if #password < 8 then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400 text-sm mt-2">Password must be at least 8 characters</div>')
    end
    self:set_flash("error", "Password must be at least 8 characters")
    return self:redirect("/auth/signup?token=" .. token .. "&redirect=" .. redirect_to)
  end

  -- Check if email exists
  local existingRes = db:Sdbql("FOR u IN users FILTER u.email == @email LIMIT 1 RETURN u._key", { email = email })
  if existingRes and existingRes.result and #existingRes.result > 0 then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400 text-sm mt-2">Email already registered</div>')
    end
    self:set_flash("error", "Email already registered")
    return self:redirect("/auth/signup?token=" .. token .. "&redirect=" .. redirect_to)
  end

  -- Hash password
  local salt = GetRandomBytes(32)
  local hash, err = argon2.hash_encoded(password, salt)
  if err then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400 text-sm mt-2">Registration failed, please try again</div>')
    end
    self:set_flash("error", "Registration failed")
    return self:redirect("/auth/signup?token=" .. token .. "&redirect=" .. redirect_to)
  end

  -- Create user
  local user = {
    firstname = firstname,
    lastname = lastname,
    email = email,
    password_hash = hash,
    connection_count = 0,
    status = "offline",
    is_admin = is_first_user, -- First user is admin
    created_at = os.time()
  }

  local insertRes = db:Sdbql("INSERT @user INTO users RETURN NEW", { user = user })
  if not insertRes or not insertRes.result or #insertRes.result == 0 then
    if self:is_htmx_request() then
      return self:html('<div class="text-red-400 text-sm mt-2">Registration failed, please try again</div>')
    end
    self:set_flash("error", "Registration failed")
    return self:redirect("/auth/signup?token=" .. token .. "&redirect=" .. redirect_to)
  end

  local newUser = insertRes.result[1]

  -- Mark invitation token as used
  if invitation then
    invitation:mark_used()
  end

  -- Set session (24 hours = 1440 minutes)
  self:set_session({
    user_id = newUser._id,
    user_data = {
      _id = newUser._id,
      _key = newUser._key,
      email = newUser.email,
      firstname = newUser.firstname,
      lastname = newUser.lastname,
      is_admin = newUser.is_admin or is_first_user,
      channel_last_seen = {}
    }
  }, 24 * 60)

  -- HTMX redirect
  if self:is_htmx_request() then
    self:set_header("HX-Redirect", redirect_to)
    return self:html("")
  end

  return self:redirect(redirect_to)
end

-- Logout (GET /auth/logout)
function AuthController:logout()
  DestroySession()
  local redirect_to = self.params.redirect or "/auth/login"
  return self:redirect(redirect_to)
end

return AuthController
