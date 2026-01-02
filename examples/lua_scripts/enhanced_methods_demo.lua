-- SoliDB Enhanced Lua Methods Example
-- This script demonstrates the new enhanced Lua methods available in SoliDB

function example_validation()
    -- Define a user validation schema
    local user_schema = {
        type = "object",
        properties = {
            name = { type = "string", minLength = 1 },
            email = { type = "string", format = "email" },
            age = { type = "number", minimum = 0, maximum = 120 }
        },
        required = {"name", "email"}
    }
    
    -- Test data
    local valid_user = {
        name = "Alice Smith",
        email = "alice@example.com",
        age = 30
    }
    
    local invalid_user = {
        name = "",
        email = "not-an-email",
        age = 150
    }
    
    -- Validate data
    local is_valid = solidb.validate(valid_user, user_schema)
    local validation_result = solidb.validate_detailed(invalid_user, user_schema)
    
    return {
        valid_user_ok = is_valid,
        invalid_user_errors = validation_result.errors,
        error_count = validation_result.error_count
    }
end

function example_sanitization()
    -- Sanitize user input
    local dirty_input = {
        name = "  Alice Smith  ",
        email = "ALICE@EXAMPLE.COM",
        message = "  Hello    World  ",
        html = "<script>alert('xss')</script>Hello"
    }
    
    local clean_input = solidb.sanitize(dirty_input, {
        trim = true,
        lowercase = {"email"},
        normalize_whitespace = true,
        strip_html = true
    })
    
    return {
        original = dirty_input,
        cleaned = clean_input
    }
end

function example_error_handling()
    -- Demonstrate error handling patterns
    local function risky_operation(user_data)
        solidb.assert(user_data.email, "Email is required")
        solidb.assert(user_data.age >= 18, "User must be 18 or older")
        return { status = "success", user = user_data }
    end
    
    -- Try-catch pattern
    local result = solidb.try(risky_operation, function(error)
        return { 
            status = "error", 
            message = "Validation failed: " .. error,
            handled = true
        }
    end)
    
    return result
end

function example_http_helpers()
    -- Demonstrate HTTP helpers
    local cache_key = "user:profile:" .. request.query.user_id
    local cached_profile = solidb.cache_get(cache_key)
    
    if cached_profile then
        return { profile = cached_profile, from_cache = true }
    end
    
    -- Fetch fresh data (simulated)
    local profile = {
        id = request.query.user_id,
        name = "John Doe",
        last_login = solidb.now()
    }
    
    -- Cache for 1 hour
    solidb.cache(cache_key, profile, 3600)
    
    -- Set authentication cookie
    solidb.set_cookie("auth_token", "abc123xyz", {
        expires = time.iso(time.add(time.now(), 24, "h")), -- 24 hours from now
        secure = true,
        httpOnly = true,
        sameSite = "Strict"
    })
    
    return { profile = profile, from_cache = false }
end

function example_type_checking()
    -- Demonstrate enhanced type checking
    local test_values = {
        "string",
        42,
        true,
        { nested = "table" },
        function() return "test" end,
        nil
    }
    
    local types = {}
    for i, value in ipairs(test_values) do
        types[i] = solidb.typeof(value)
    end
    
    return {
        original_values = test_values,
        types = types
    }
end

function example_response_formats()
    -- Different response formats based on request
    local format = request.query.format or "json"
    
    if format == "html" then
        return response.html([[
            <!DOCTYPE html>
            <html>
            <head><title>Hello</title></head>
            <body><h1>Hello from SoliDB Lua!</h1></body>
            </html>
        ]])
    elseif format == "file" then
        return response.file("/path/to/file.pdf")
    elseif format == "download" then
        return response.stream({"chunk1", "chunk2", "chunk3"})
    else
        -- Set CORS headers for API
        response.cors({
            origins = {"https://app.example.com"},
            methods = {"GET", "POST", "PUT"},
            credentials = true
        })
        
        return response.json({
            message = "Hello from SoliDB!",
            timestamp = time.iso(),
            request_method = request.method,
            request_path = request.path
        })
    end
end

-- Main handler - choose example based on path
if request.path:match("validation") then
    return example_validation()
elseif request.path:match("sanitization") then
    return example_sanitization()
elseif request.path:match("error") then
    return example_error_handling()
elseif request.path:match("http") then
    return example_http_helpers()
elseif request.path:match("types") then
    return example_type_checking()
elseif request.path:match("response") then
    return example_response_formats()
else
    return response.json({
        available_examples = {
            "/validation" = "JSON schema validation examples",
            "/sanitization" = "Input sanitization examples", 
            "/error" = "Error handling patterns",
            "/http" = "HTTP helpers demonstration",
            "/types" = "Enhanced type checking",
            "/response" = "Different response formats"
        },
        message = "Add ?format=html|file|download to response example"
    })
end