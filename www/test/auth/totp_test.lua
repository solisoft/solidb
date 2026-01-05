-- Tests for TOTP module
-- test/auth/totp_test.lua

package.path = package.path .. ";.lua/?.lua"

local Test = require("test")
local describe, it, expect = Test.describe, Test.it, Test.expect

-- Mock state - accessible from tests
_G._test_mock_time = _G._test_mock_time or 1735660800 -- 2025-01-01 00:00:00 UTC
local random_counter = 0

-- Store original os.time and wrap it to handle both modes
local original_os_time = os.time
os.time = function(t)
  if t then
    -- Table argument - use original function to convert date to timestamp
    return original_os_time(t)
  else
    -- No argument - return mocked time
    return _G._test_mock_time
  end
end

-- Mock redbean crypto functions BEFORE loading TOTP module
_G.DecodeBase32 = _G.DecodeBase32 or function(str, alphabet)
  alphabet = alphabet or "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
  local result = {}
  local buffer = 0
  local bits = 0

  for i = 1, #str do
    local c = str:sub(i, i)
    local val = alphabet:find(c, 1, true)
    if val then
      val = val - 1
      buffer = buffer * 32 + val
      bits = bits + 5
      while bits >= 8 do
        bits = bits - 8
        local byte = math.floor(buffer / (2 ^ bits)) % 256
        table.insert(result, string.char(byte))
      end
    end
  end

  return table.concat(result)
end

_G.EncodeBase32 = _G.EncodeBase32 or function(str, alphabet)
  alphabet = alphabet or "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
  local result = {}
  local buffer = 0
  local bits = 0

  for i = 1, #str do
    buffer = buffer * 256 + str:byte(i)
    bits = bits + 8
    while bits >= 5 do
      bits = bits - 5
      local idx = math.floor(buffer / (2 ^ bits)) % 32 + 1
      table.insert(result, alphabet:sub(idx, idx))
    end
  end

  if bits > 0 then
    buffer = buffer * (2 ^ (5 - bits))
    local idx = buffer % 32 + 1
    table.insert(result, alphabet:sub(idx, idx))
  end

  return table.concat(result)
end

_G.GetCryptoHash = _G.GetCryptoHash or function(algorithm, message, key)
  -- Simple HMAC mock for testing - returns predictable hash
  local hash = {}
  local size = algorithm == "SHA256" and 32 or 20
  for i = 1, size do
    local byte = (message:byte((i % #message) + 1) or 0) +
                 (key:byte((i % #key) + 1) or 0) + i
    hash[i] = string.char(byte % 256)
  end
  return table.concat(hash)
end

_G.GetRandomBytes = _G.GetRandomBytes or function(length)
  -- Generate unique pseudo-random bytes for testing using counter
  random_counter = random_counter + 1
  local result = {}
  for i = 1, length do
    result[i] = string.char((random_counter * 17 + i * 31) % 256)
  end
  return table.concat(result)
end

-- Load TOTP module AFTER mocks are set up
local TOTP = require("totp")

-- Helper to reset mock state
local function reset_mocks()
  _G._test_mock_time = 1735660800
  random_counter = 0
end

describe("TOTP GenerateSecret", function()
  it("should generate a Base32-encoded secret", function()
    reset_mocks()
    local secret = TOTP.GenerateSecret()
    expect.truthy(secret)
    expect.eq(type(secret), "string")
    -- Base32 only contains A-Z and 2-7
    expect.truthy(secret:match("^[A-Z2-7]+$"))
  end)

  it("should generate different secrets on each call", function()
    reset_mocks()
    local secret1 = TOTP.GenerateSecret()
    local secret2 = TOTP.GenerateSecret()

    expect.neq(secret1, secret2)
  end)

  it("should accept custom length parameter", function()
    reset_mocks()
    local short_secret = TOTP.GenerateSecret(10)
    local long_secret = TOTP.GenerateSecret(32)

    expect.truthy(short_secret)
    expect.truthy(long_secret)
    -- Longer input should produce longer Base32 output
    expect.truthy(#short_secret < #long_secret)
  end)
end)

describe("TOTP Generate", function()
  it("should generate a 6-digit code by default", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local code = TOTP.Generate(secret)

    expect.truthy(code)
    expect.eq(#code, 6)
    expect.truthy(code:match("^%d+$"))
  end)

  it("should generate codes with custom digit count", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local code = TOTP.Generate(secret, 8)

    expect.eq(#code, 8)
    expect.truthy(code:match("^%d+$"))
  end)

  it("should generate consistent codes for same time", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local code1 = TOTP.Generate(secret)
    local code2 = TOTP.Generate(secret)

    expect.eq(code1, code2)
  end)
end)

describe("TOTP Validate", function()
  it("should validate correct code", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local code = TOTP.Generate(secret)

    local result = TOTP.Validate(secret, code)
    expect.truthy(result)
  end)

  it("should reject incorrect code", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"

    local result = TOTP.Validate(secret, "000000")
    expect.falsy(result)
  end)

  it("should accept code from previous period (time drift)", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"

    -- Generate code at time T
    _G._test_mock_time = 1735660800
    local code = TOTP.Generate(secret)

    -- Validate at time T+30 (should still accept previous period)
    _G._test_mock_time = 1735660830
    local result = TOTP.Validate(secret, code)
    expect.truthy(result)
  end)

  it("should reject code with wrong length", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"

    local result = TOTP.Validate(secret, "12345") -- 5 digits
    expect.falsy(result)

    result = TOTP.Validate(secret, "1234567") -- 7 digits
    expect.falsy(result)
  end)

  it("should strip spaces from user input", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local code = TOTP.Generate(secret)

    -- Add spaces to code
    local spaced_code = code:sub(1, 3) .. " " .. code:sub(4, 6)
    local result = TOTP.Validate(secret, spaced_code)
    expect.truthy(result)
  end)
end)

describe("TOTP GetTimeRemaining", function()
  it("should return seconds remaining in current period", function()
    _G._test_mock_time = 1735660800 -- Exactly at period boundary (divisible by 30)
    local remaining = TOTP.GetTimeRemaining()
    expect.eq(remaining, 30)
  end)

  it("should return correct value mid-period", function()
    _G._test_mock_time = 1735660815 -- 15 seconds into period
    local remaining = TOTP.GetTimeRemaining()
    expect.eq(remaining, 15)
  end)

  it("should return 1 second at end of period", function()
    _G._test_mock_time = 1735660829 -- 29 seconds into period
    local remaining = TOTP.GetTimeRemaining()
    expect.eq(remaining, 1)
  end)
end)

describe("TOTP GenerateRecoveryCodes", function()
  it("should generate default 10 codes", function()
    reset_mocks()
    local codes = TOTP.GenerateRecoveryCodes()

    expect.eq(#codes, 10)
  end)

  it("should generate custom number of codes", function()
    reset_mocks()
    local codes = TOTP.GenerateRecoveryCodes(5)

    expect.eq(#codes, 5)
  end)

  it("should generate codes in XXXX-XXXX format", function()
    reset_mocks()
    local codes = TOTP.GenerateRecoveryCodes(1)

    expect.truthy(codes[1]:match("^%x%x%x%x%-%x%x%x%x$"))
  end)

  it("should generate unique codes", function()
    -- Reset counter to ensure consistent test
    random_counter = 100
    local codes = TOTP.GenerateRecoveryCodes(10)
    local seen = {}

    for _, code in ipairs(codes) do
      expect.nil_value(seen[code])
      seen[code] = true
    end
  end)
end)

describe("TOTP GetAuthURI", function()
  it("should generate valid otpauth URI", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local uri = TOTP.GetAuthURI(secret, "MyApp", "user@example.com")

    expect.truthy(uri:match("^otpauth://totp/"))
    expect.truthy(uri:match("secret=" .. secret))
    expect.truthy(uri:match("issuer=MyApp"))
    expect.truthy(uri:match("digits=6"))
    expect.truthy(uri:match("period=30"))
  end)

  it("should URL-encode special characters in issuer", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local uri = TOTP.GetAuthURI(secret, "My App & Co", "user@example.com")

    expect.truthy(uri:match("My%%20App"))
    expect.truthy(uri:match("%%26"))
  end)

  it("should URL-encode special characters in account", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local uri = TOTP.GetAuthURI(secret, "MyApp", "user+test@example.com")

    expect.truthy(uri:match("user%%2Btest"))
  end)
end)

-- ============================================================================
-- Security Features Tests
-- ============================================================================

describe("TOTP ValidateOnce (replay prevention)", function()
  it("should validate correct code and return counter", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local code = TOTP.Generate(secret)

    local valid, counter, drift = TOTP.ValidateOnce(secret, code, 0)

    expect.truthy(valid)
    expect.truthy(counter)
    expect.eq(drift, 0)
  end)

  it("should reject replayed code with same counter", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local code = TOTP.Generate(secret)

    -- First validation succeeds
    local valid1, counter1 = TOTP.ValidateOnce(secret, code, 0)
    expect.truthy(valid1)

    -- Second validation with same code should fail (replay)
    local valid2 = TOTP.ValidateOnce(secret, code, counter1)
    expect.falsy(valid2)
  end)

  it("should reject code with counter less than last used", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"
    local code = TOTP.Generate(secret)

    -- Use a very high last_used_counter
    local valid = TOTP.ValidateOnce(secret, code, 999999999)
    expect.falsy(valid)
  end)

  it("should return drift value for time window", function()
    reset_mocks()
    local secret = "JBSWY3DPEHPK3PXP"

    -- Generate code at current time
    _G._test_mock_time = 1735660800
    local code = TOTP.Generate(secret)

    -- Validate at T+30 (previous period should match with drift -1)
    _G._test_mock_time = 1735660830
    local valid, _, drift = TOTP.ValidateOnce(secret, code, 0)

    expect.truthy(valid)
    expect.eq(drift, -1)
  end)
end)

describe("TOTP HashRecoveryCode", function()
  it("should hash recovery code", function()
    reset_mocks()
    local hash = TOTP.HashRecoveryCode("A1B2-C3D4")

    expect.truthy(hash)
    expect.eq(type(hash), "string")
    -- SHA256 produces 64 hex characters
    expect.eq(#hash, 64)
  end)

  it("should normalize input (case insensitive)", function()
    reset_mocks()
    local hash1 = TOTP.HashRecoveryCode("a1b2-c3d4")
    local hash2 = TOTP.HashRecoveryCode("A1B2-C3D4")

    expect.eq(hash1, hash2)
  end)

  it("should normalize input (ignore dashes and spaces)", function()
    reset_mocks()
    local hash1 = TOTP.HashRecoveryCode("A1B2-C3D4")
    local hash2 = TOTP.HashRecoveryCode("A1B2C3D4")
    local hash3 = TOTP.HashRecoveryCode("A1B2 C3D4")

    expect.eq(hash1, hash2)
    expect.eq(hash2, hash3)
  end)
end)

describe("TOTP ValidateRecoveryCode", function()
  it("should validate correct recovery code", function()
    reset_mocks()
    random_counter = 50 -- Ensure unique codes

    local codes, hashes = TOTP.GenerateRecoveryCodesWithHashes(5)

    -- Validate the first code
    local valid, index = TOTP.ValidateRecoveryCode(codes[1], hashes)

    expect.truthy(valid)
    expect.eq(index, 1)
  end)

  it("should return correct index for matched code", function()
    reset_mocks()
    random_counter = 60

    local codes, hashes = TOTP.GenerateRecoveryCodesWithHashes(5)

    -- Validate the third code
    local valid, index = TOTP.ValidateRecoveryCode(codes[3], hashes)

    expect.truthy(valid)
    expect.eq(index, 3)
  end)

  it("should reject invalid recovery code", function()
    reset_mocks()
    random_counter = 70

    local _, hashes = TOTP.GenerateRecoveryCodesWithHashes(5)

    local valid, index = TOTP.ValidateRecoveryCode("XXXX-YYYY", hashes)

    expect.falsy(valid)
    expect.nil_value(index)
  end)

  it("should handle nil inputs", function()
    reset_mocks()

    local valid1 = TOTP.ValidateRecoveryCode(nil, {})
    local valid2 = TOTP.ValidateRecoveryCode("code", nil)

    expect.falsy(valid1)
    expect.falsy(valid2)
  end)
end)

describe("TOTP GenerateRecoveryCodesWithHashes", function()
  it("should return codes and hashes", function()
    reset_mocks()
    random_counter = 80

    local codes, hashes = TOTP.GenerateRecoveryCodesWithHashes(5)

    expect.eq(#codes, 5)
    expect.eq(#hashes, 5)
  end)

  it("should generate valid hash for each code", function()
    reset_mocks()
    random_counter = 90

    local codes, hashes = TOTP.GenerateRecoveryCodesWithHashes(3)

    for i, code in ipairs(codes) do
      local expected_hash = TOTP.HashRecoveryCode(code)
      expect.eq(hashes[i], expected_hash)
    end
  end)
end)

describe("TOTP RateLimiter", function()
  it("should not lock out with few attempts", function()
    reset_mocks()
    local limiter = TOTP.CreateRateLimiter(5, 300)

    local locked = limiter:is_locked(3, nil)

    expect.falsy(locked)
  end)

  it("should lock out after max attempts", function()
    reset_mocks()
    local limiter = TOTP.CreateRateLimiter(5, 300)

    local locked, remaining = limiter:is_locked(5, _G._test_mock_time)

    expect.truthy(locked)
    expect.eq(remaining, 300)
  end)

  it("should unlock after lockout expires", function()
    reset_mocks()
    local limiter = TOTP.CreateRateLimiter(5, 300)

    -- Failed 5 times, 301 seconds ago
    local last_failed = _G._test_mock_time - 301

    local locked = limiter:is_locked(5, last_failed)

    expect.falsy(locked)
  end)

  it("should return remaining lockout time", function()
    reset_mocks()
    local limiter = TOTP.CreateRateLimiter(5, 300)

    -- Failed 5 times, 100 seconds ago
    local last_failed = _G._test_mock_time - 100

    local locked, remaining = limiter:is_locked(5, last_failed)

    expect.truthy(locked)
    expect.eq(remaining, 200)
  end)

  it("should record failure", function()
    reset_mocks()
    local limiter = TOTP.CreateRateLimiter(5, 300)

    local new_attempts, timestamp = limiter:record_failure(2)

    expect.eq(new_attempts, 3)
    expect.eq(timestamp, _G._test_mock_time)
  end)

  it("should reset after success", function()
    reset_mocks()
    local limiter = TOTP.CreateRateLimiter(5, 300)

    local attempts, timestamp = limiter:reset()

    expect.eq(attempts, 0)
    expect.nil_value(timestamp)
  end)
end)

return Test
