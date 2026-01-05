-- Luaonbeans Test Framework
-- A simple testing framework inspired by busted/luaunit

local Test = {}
Test.__index = Test

-- Test state
local tests = {}
local current_describe = nil
local results = { passed = 0, failed = 0, errors = {} }
local before_hooks = {}
local after_hooks = {}

-- Colors for output
local colors = {
  reset = "\27[0m",
  green = "\27[32m",
  red = "\27[31m",
  yellow = "\27[33m",
  cyan = "\27[36m"
}

-- Describe a test suite
function Test.describe(name, fn)
  local parent = current_describe
  if parent then
    name = parent .. " > " .. name
  end

  current_describe = name
  tests[name] = tests[name] or {}
  before_hooks[name] = {}
  after_hooks[name] = {}
  fn()
  current_describe = parent
end

-- Define a test
function Test.it(name, fn)
  if current_describe then
    table.insert(tests[current_describe], { name = name, fn = fn })
  else
    tests["_root"] = tests["_root"] or {}
    table.insert(tests["_root"], { name = name, fn = fn })
  end
end

-- Before hook - runs before each test in describe block
function Test.before(fn)
  if current_describe then
    table.insert(before_hooks[current_describe], fn)
  end
end

-- After hook - runs after each test in describe block
function Test.after(fn)
  if current_describe then
    table.insert(after_hooks[current_describe], fn)
  end
end

-- Assertions
Test.expect = {}

function Test.expect.eq(actual, expected, message)
  if actual ~= expected then
    error(string.format(
      "%sExpected %s to equal %s",
      message and (message .. ": ") or "",
      tostring(actual),
      tostring(expected)
    ), 2)
  end
end

function Test.expect.neq(actual, expected, message)
  if actual == expected then
    error(string.format(
      "%sExpected %s to not equal %s",
      message and (message .. ": ") or "",
      tostring(actual),
      tostring(expected)
    ), 2)
  end
end

function Test.expect.truthy(value, message)
  if not value then
    error(string.format(
      "%sExpected value to be truthy, got %s",
      message and (message .. ": ") or "",
      tostring(value)
    ), 2)
  end
end

function Test.expect.falsy(value, message)
  if value then
    error(string.format(
      "%sExpected value to be falsy, got %s",
      message and (message .. ": ") or "",
      tostring(value)
    ), 2)
  end
end

function Test.expect.nil_value(value, message)
  if value ~= nil then
    error(string.format(
      "%sExpected nil, got %s",
      message and (message .. ": ") or "",
      tostring(value)
    ), 2)
  end
end

function Test.expect.not_nil(value, message)
  if value == nil then
    error((message or "Expected value to not be nil"), 2)
  end
end

function Test.expect.contains(tbl, value, message)
  local found = false
  for _, v in pairs(tbl) do
    if v == value then
      found = true
      break
    end
  end
  if not found then
    error(string.format(
      "%sExpected table to contain %s",
      message and (message .. ": ") or "",
      tostring(value)
    ), 2)
  end
end

function Test.expect.has_key(tbl, key, message)
  if tbl[key] == nil then
    error(string.format(
      "%sExpected table to have key '%s'",
      message and (message .. ": ") or "",
      tostring(key)
    ), 2)
  end
end

function Test.expect.matches(actual, pattern, message)
  if not string.match(actual, pattern) then
    error(string.format(
      "%sExpected '%s' to match pattern '%s'",
      message and (message .. ": ") or "",
      tostring(actual),
      pattern
    ), 2)
  end
end

function Test.expect.error(fn, message)
  local ok = pcall(fn)
  if ok then
    error((message or "Expected function to throw an error"), 2)
  end
end

function Test.expect.no_error(fn, message)
  local ok, err = pcall(fn)
  if not ok then
    error(string.format(
      "%sExpected no error, got: %s",
      message and (message .. ": ") or "",
      tostring(err)
    ), 2)
  end
end

-- Run all tests
function Test.run()
  print(colors.cyan .. "\n🧪 Running tests...\n" .. colors.reset)

  results = { passed = 0, failed = 0, errors = {} }

  -- Get and sort suite names
  local suite_names = {}
  for name, _ in pairs(tests) do
    table.insert(suite_names, name)
  end
  table.sort(suite_names)

  for _, suite_name in ipairs(suite_names) do
    local suite_tests = tests[suite_name]
    
    -- Skip empty suites
    if #suite_tests > 0 then
      if suite_name ~= "_root" then
        print(colors.yellow .. "  " .. suite_name .. colors.reset)
      end
  
      for _, test in ipairs(suite_tests) do
        -- Run before hooks
        local before_ok = true
        if before_hooks[suite_name] then
          for _, hook in ipairs(before_hooks[suite_name]) do
            local ok, err = pcall(hook)
            if not ok then
              before_ok = false
              results.failed = results.failed + 1
              table.insert(results.errors, {
                suite = suite_name,
                test = test.name,
                error = "Before hook error: " .. tostring(err)
              })
              print(colors.red .. "    ✗ " .. test.name .. " (before hook failed)" .. colors.reset)
              break
            end
          end
        end
  
        if before_ok then
          local ok, err = pcall(test.fn)
          if ok then
            results.passed = results.passed + 1
            print(colors.green .. "    ✓ " .. test.name .. colors.reset)
          else
            results.failed = results.failed + 1
            table.insert(results.errors, {
              suite = suite_name,
              test = test.name,
              error = err
            })
            print(colors.red .. "    ✗ " .. test.name .. colors.reset)
          end
        end
  
        -- Run after hooks
        if after_hooks[suite_name] then
          for _, hook in ipairs(after_hooks[suite_name]) do
            local ok, err = pcall(hook)
            if not ok then
              print(colors.red .. "    ⚠ After hook error: " .. tostring(err) .. colors.reset)
            end
          end
        end
      end
    end
  end

  -- Print summary
  print("")
  if results.failed == 0 then
    print(colors.green .. string.format(
      "✓ All %d tests passed!",
      results.passed
    ) .. colors.reset)
  else
    print(colors.red .. string.format(
      "✗ %d of %d tests failed",
      results.failed,
      results.passed + results.failed
    ) .. colors.reset)

    -- Print errors
    print(colors.red .. "\nFailures:" .. colors.reset)
    for _, e in ipairs(results.errors) do
      print(string.format("  %s > %s", e.suite, e.test))
      print(colors.red .. "    " .. e.error .. colors.reset)
    end
  end

  print("")

  -- Return exit code
  return results.failed == 0 and 0 or 1
end

-- Clear all tests (useful between test runs)
function Test.clear()
  tests = {}
  results = { passed = 0, failed = 0, errors = {} }
end

-- Aliases for convenience
Test.test = Test.it
Test.context = Test.describe

return Test
