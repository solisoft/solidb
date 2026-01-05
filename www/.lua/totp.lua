-- TOTP (Time-based One-Time Password) implementation
-- RFC 6238 compliant

local TOTP = {}

local BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
local DEFAULT_DIGITS = 6
local DEFAULT_PERIOD = 30

-- Get current time counter (30-second intervals)
local function getTimeCounter(offset)
  local current_time = os.time() + (offset or 0)
  return math.floor(current_time / DEFAULT_PERIOD)
end

-- Convert byte to integer
local function byte_to_int(byte)
  return byte:byte()
end

-- RFC 4226 dynamic truncation
local function dynamic_truncation(hash)
  local offset = byte_to_int(hash:sub(20, 20)) % 16 + 1

  local part1 = byte_to_int(hash:sub(offset, offset)) * 2^24
  local part2 = byte_to_int(hash:sub(offset + 1, offset + 1)) * 2^16
  local part3 = byte_to_int(hash:sub(offset + 2, offset + 2)) * 2^8
  local part4 = byte_to_int(hash:sub(offset + 3, offset + 3))

  local binary_code = part1 + part2 + part3 + part4
  return binary_code % 2^31
end

-- Generate TOTP code for a given time counter
local function generateTOTP(secret, digits, time_counter)
  local key = DecodeBase32(secret, BASE32_ALPHABET)
  local message = string.pack(">I8", time_counter)
  local hash = GetCryptoHash("SHA1", message, key)
  local binary_code = dynamic_truncation(hash)
  local otp = binary_code % (10 ^ digits)
  return string.format("%0" .. digits .. "d", otp)
end

-- Timing-safe string comparison to prevent timing attacks
local function secure_compare(a, b)
  if type(a) ~= "string" or type(b) ~= "string" then
    return false
  end
  if #a ~= #b then
    return false
  end
  local result = 0
  for i = 1, #a do
    result = result + (string.byte(a, i) ~ string.byte(b, i))
  end
  return result == 0
end

-- URL encode a string
local function url_encode(str)
  if not str then return "" end
  str = string.gsub(str, "([^%w%-_.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
  return str
end

---Generate a new random Base32-encoded secret
---@param length number? Number of bytes for the secret (default 20 = 160 bits)
---@return string Base32-encoded secret
function TOTP.GenerateSecret(length)
  length = length or 20
  local bytes = GetRandomBytes(length)
  return EncodeBase32(bytes, BASE32_ALPHABET)
end

---Validate a user-provided TOTP code
---@param secret string Base32-encoded secret
---@param user_otp string User-provided OTP code
---@param digits number? Number of digits (default 6)
---@return boolean True if valid
function TOTP.Validate(secret, user_otp, digits)
  digits = digits or DEFAULT_DIGITS

  -- Normalize user input (remove spaces, ensure string)
  user_otp = tostring(user_otp):gsub("%s", "")

  -- Check length
  if #user_otp ~= digits then
    return false
  end

  -- Check time windows: previous, current, and next 30-second period
  local time_steps = {-1, 0, 1}

  for _, step in ipairs(time_steps) do
    local time_counter = getTimeCounter(step * DEFAULT_PERIOD)
    local generated_otp = generateTOTP(secret, digits, time_counter)

    if generated_otp == user_otp then
      return true
    end
  end

  return false
end

---Generate the current TOTP code (for testing/display)
---@param secret string Base32-encoded secret
---@param digits number? Number of digits (default 6)
---@return string Current OTP code
function TOTP.Generate(secret, digits)
  digits = digits or DEFAULT_DIGITS
  return generateTOTP(secret, digits, getTimeCounter(0))
end

---Generate random recovery codes
---Recovery codes are random, not time-based (unlike TOTP)
---Store these hashed in your database
---@param count number? Number of codes to generate (default 10)
---@return table Array of recovery codes
function TOTP.GenerateRecoveryCodes(count)
  count = count or 10
  local codes = {}

  for i = 1, count do
    -- Generate 8 random bytes, encode as hex, split into groups
    local bytes = GetRandomBytes(4)
    local hex = ""
    for j = 1, #bytes do
      hex = hex .. string.format("%02x", bytes:byte(j))
    end
    -- Format as XXXX-XXXX for readability
    codes[i] = hex:sub(1, 4):upper() .. "-" .. hex:sub(5, 8):upper()
  end

  return codes
end

---Generate an OTP Auth URI for QR codes
---@param secret string Base32-encoded secret
---@param issuer string Your app/company name
---@param account string User's email or username
---@return string otpauth:// URI
function TOTP.GetAuthURI(secret, issuer, account)
  local encoded_issuer = url_encode(issuer)
  local encoded_account = url_encode(account)

  return string.format(
    "otpauth://totp/%s:%s?secret=%s&issuer=%s&digits=%d&period=%d",
    encoded_issuer,
    encoded_account,
    secret,  -- Already Base32 encoded
    encoded_issuer,
    DEFAULT_DIGITS,
    DEFAULT_PERIOD
  )
end

---Get seconds remaining until current code expires
---@return number Seconds remaining (0-30)
function TOTP.GetTimeRemaining()
  return DEFAULT_PERIOD - (os.time() % DEFAULT_PERIOD)
end

-- ============================================================================
-- Security Features
-- ============================================================================

---Validate TOTP with replay attack prevention
---Prevents the same code from being used twice
---@param secret string Base32-encoded secret
---@param user_otp string User-provided OTP code
---@param last_used_counter number? Last successfully used time counter (from DB)
---@param digits number? Number of digits (default 6)
---@return boolean valid True if valid and not replayed
---@return number? counter The time counter used (store in DB for next validation)
---@return number? drift Time drift (-1, 0, or 1) for logging
function TOTP.ValidateOnce(secret, user_otp, last_used_counter, digits)
  digits = digits or DEFAULT_DIGITS
  last_used_counter = last_used_counter or 0

  -- Normalize user input
  user_otp = tostring(user_otp):gsub("%s", "")

  if #user_otp ~= digits then
    return false, nil, nil
  end

  -- Check time windows: previous, current, and next
  local time_steps = {-1, 0, 1}

  for _, step in ipairs(time_steps) do
    local time_counter = getTimeCounter(step * DEFAULT_PERIOD)
    local generated_otp = generateTOTP(secret, digits, time_counter)

    if generated_otp == user_otp then
      -- Check for replay attack
      if time_counter <= last_used_counter then
        return false, nil, nil -- Code already used
      end
      return true, time_counter, step
    end
  end

  return false, nil, nil
end

---Hash a recovery code for secure storage
---@param code string Plain recovery code
---@return string Hashed code (hex-encoded SHA256)
function TOTP.HashRecoveryCode(code)
  -- Normalize: uppercase, remove dashes/spaces
  code = tostring(code):upper():gsub("[%s%-]", "")
  local hash = GetCryptoHash("SHA256", code, "")
  -- Convert to hex
  local hex = ""
  for i = 1, #hash do
    hex = hex .. string.format("%02x", hash:byte(i))
  end
  return hex
end

---Validate a recovery code against stored hashes
---Uses timing-safe comparison
---@param user_code string User-provided recovery code
---@param hashed_codes table Array of hashed codes from DB
---@return boolean valid True if code matches
---@return number? index Index of matched code (to mark as used)
function TOTP.ValidateRecoveryCode(user_code, hashed_codes)
  if not user_code or not hashed_codes then
    return false, nil
  end

  local user_hash = TOTP.HashRecoveryCode(user_code)
  local matched_index = nil

  -- Timing-safe: always check all codes
  for i, stored_hash in ipairs(hashed_codes) do
    if secure_compare(user_hash, stored_hash) then
      matched_index = i
    end
  end

  return matched_index ~= nil, matched_index
end

---Generate recovery codes with their hashes
---@param count number? Number of codes (default 10)
---@return table codes Plain codes to show user once
---@return table hashes Hashed codes to store in DB
function TOTP.GenerateRecoveryCodesWithHashes(count)
  local codes = TOTP.GenerateRecoveryCodes(count)
  local hashes = {}

  for i, code in ipairs(codes) do
    hashes[i] = TOTP.HashRecoveryCode(code)
  end

  return codes, hashes
end

-- ============================================================================
-- Rate Limiting
-- ============================================================================

---Create a rate limiter for TOTP validation
---@param max_attempts number Maximum failed attempts before lockout
---@param lockout_seconds number Lockout duration in seconds
---@return table Rate limiter instance
function TOTP.CreateRateLimiter(max_attempts, lockout_seconds)
  return {
    max_attempts = max_attempts or 5,
    lockout_seconds = lockout_seconds or 300, -- 5 minutes default

    ---Check if user is locked out
    ---@param failed_attempts number Current failed attempt count
    ---@param last_failed_at number? Timestamp of last failure
    ---@return boolean locked True if locked out
    ---@return number? seconds_remaining Seconds until unlock (if locked)
    is_locked = function(self, failed_attempts, last_failed_at)
      if failed_attempts < self.max_attempts then
        return false, nil
      end

      if not last_failed_at then
        return true, self.lockout_seconds
      end

      local elapsed = os.time() - last_failed_at
      if elapsed >= self.lockout_seconds then
        return false, nil -- Lockout expired
      end

      return true, self.lockout_seconds - elapsed
    end,

    ---Record a failed attempt
    ---@param current_attempts number Current failed count
    ---@return number new_attempts Updated count
    ---@return number timestamp Current timestamp
    record_failure = function(self, current_attempts)
      return (current_attempts or 0) + 1, os.time()
    end,

    ---Reset after successful validation
    ---@return number attempts Reset to 0
    ---@return nil timestamp Clear timestamp
    reset = function(self)
      return 0, nil
    end
  }
end

return TOTP

--[[
Example usage:

local TOTP = require("totp")

-- ============================================================================
-- SETUP (when user enables 2FA)
-- ============================================================================

-- Generate secret for new user
local secret = TOTP.GenerateSecret()
-- Save `secret` to user record in database

-- Generate QR code URI for authenticator apps
local uri = TOTP.GetAuthURI(secret, "MyApp", "user@example.com")
-- Render this URI as a QR code

-- Generate recovery codes (save hashed versions in DB)
local codes, hashes = TOTP.GenerateRecoveryCodesWithHashes(10)
-- Show `codes` to user ONCE, store `hashes` in database

-- ============================================================================
-- LOGIN (with replay attack prevention)
-- ============================================================================

-- Create rate limiter (5 attempts, 5 minute lockout)
local limiter = TOTP.CreateRateLimiter(5, 300)

-- Check if user is locked out (load from DB)
local failed_attempts = user.totp_failed_attempts or 0
local last_failed_at = user.totp_last_failed_at

local locked, seconds_remaining = limiter:is_locked(failed_attempts, last_failed_at)
if locked then
  print("Too many attempts. Try again in " .. seconds_remaining .. " seconds")
  return
end

-- Validate with replay prevention
local user_code = "123456"
local last_counter = user.totp_last_counter or 0

local valid, new_counter, drift = TOTP.ValidateOnce(secret, user_code, last_counter)

if valid then
  -- Success! Update database
  user.totp_last_counter = new_counter
  user.totp_failed_attempts, user.totp_last_failed_at = limiter:reset()
  print("2FA verified! (drift: " .. drift .. ")")
else
  -- Failed - record attempt
  user.totp_failed_attempts, user.totp_last_failed_at = limiter:record_failure(failed_attempts)
  print("Invalid code")
end

-- ============================================================================
-- RECOVERY CODE VALIDATION
-- ============================================================================

local recovery_input = "A1B2-C3D4"
local valid, index = TOTP.ValidateRecoveryCode(recovery_input, user.recovery_hashes)

if valid then
  -- Remove used code from database
  table.remove(user.recovery_hashes, index)
  print("Recovery code accepted!")
else
  print("Invalid recovery code")
end

-- ============================================================================
-- DEBUG
-- ============================================================================

print("Current code:", TOTP.Generate(secret))
print("Expires in:", TOTP.GetTimeRemaining(), "seconds")
]]
