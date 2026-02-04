//! HTTP client for Lua test scripts
//!
//! Provides an `http` module to Lua for making HTTP requests to the API endpoints.

use mlua::{Lua, Result as LuaResult, Table, Value};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// HTTP client for test scripts
pub struct TestHttpClient {
    client: Client,
    base_url: String,
    database: String,
    service: String,
    auth_token: Option<String>,
}

impl TestHttpClient {
    /// Create a new HTTP client for tests
    pub fn new(base_url: &str, database: &str, service: &str, auth_token: String) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            database: database.to_string(),
            service: service.to_string(),
            auth_token: if auth_token.is_empty() {
                None
            } else {
                Some(auth_token)
            },
        }
    }

    /// Build the full URL for a request path
    fn build_url(&self, path: &str) -> String {
        let normalized_path = path.trim_start_matches('/');

        // If path is absolute (starts with http:// or https://), use it directly
        if path.starts_with("http://") || path.starts_with("https://") {
            return path.to_string();
        }

        // Build URL: /api/{service}/{db}/{path}
        format!(
            "{}/api/{}/{}/{}",
            self.base_url, self.service, self.database, normalized_path
        )
    }

    /// Build headers for a request
    fn build_headers(&self, custom_headers: Option<HashMap<String, String>>) -> HeaderMap {
        let mut headers = HeaderMap::new();

        // Add authorization header if we have a token
        if let Some(ref token) = self.auth_token {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", token)) {
                headers.insert(AUTHORIZATION, value);
            }
        }

        // Add content-type header
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        // Add custom headers
        if let Some(custom) = custom_headers {
            for (key, value) in custom {
                if let (Ok(name), Ok(val)) = (
                    HeaderName::from_str(&key),
                    HeaderValue::from_str(&value),
                ) {
                    headers.insert(name, val);
                }
            }
        }

        headers
    }

    /// Execute a GET request
    pub fn get(
        &self,
        path: &str,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, String> {
        let url = self.build_url(path);
        let req_headers = self.build_headers(headers);

        self.client
            .get(&url)
            .headers(req_headers)
            .send()
            .map_err(|e| e.to_string())
            .and_then(HttpResponse::from_response)
    }

    /// Execute a POST request
    pub fn post(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, String> {
        let url = self.build_url(path);
        let req_headers = self.build_headers(headers);

        let mut req = self.client.post(&url).headers(req_headers);

        if let Some(body) = body {
            req = req.json(&body);
        }

        req.send()
            .map_err(|e| e.to_string())
            .and_then(HttpResponse::from_response)
    }

    /// Execute a PUT request
    pub fn put(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, String> {
        let url = self.build_url(path);
        let req_headers = self.build_headers(headers);

        let mut req = self.client.put(&url).headers(req_headers);

        if let Some(body) = body {
            req = req.json(&body);
        }

        req.send()
            .map_err(|e| e.to_string())
            .and_then(HttpResponse::from_response)
    }

    /// Execute a DELETE request
    pub fn delete(
        &self,
        path: &str,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, String> {
        let url = self.build_url(path);
        let req_headers = self.build_headers(headers);

        self.client
            .delete(&url)
            .headers(req_headers)
            .send()
            .map_err(|e| e.to_string())
            .and_then(HttpResponse::from_response)
    }

    /// Execute a PATCH request
    pub fn patch(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
        headers: Option<HashMap<String, String>>,
    ) -> Result<HttpResponse, String> {
        let url = self.build_url(path);
        let req_headers = self.build_headers(headers);

        let mut req = self.client.patch(&url).headers(req_headers);

        if let Some(body) = body {
            req = req.json(&body);
        }

        req.send()
            .map_err(|e| e.to_string())
            .and_then(HttpResponse::from_response)
    }
}

/// HTTP response wrapper
pub struct HttpResponse {
    pub status: u16,
    pub body: serde_json::Value,
    pub headers: HashMap<String, String>,
}

impl HttpResponse {
    /// Create from reqwest response
    fn from_response(response: Response) -> Result<Self, String> {
        let status = response.status().as_u16();

        // Extract headers
        let mut headers = HashMap::new();
        for (name, value) in response.headers() {
            if let Ok(v) = value.to_str() {
                headers.insert(name.to_string(), v.to_string());
            }
        }

        // Parse body as JSON, or wrap text in a string
        let text = response.text().map_err(|e| e.to_string())?;
        let body = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));

        Ok(Self {
            status,
            body,
            headers,
        })
    }
}

/// Register the HTTP module in Lua
pub fn register_http_module(lua: &Lua, client: TestHttpClient) -> LuaResult<()> {
    let globals = lua.globals();
    let http = lua.create_table()?;

    // Wrap client in Arc<Mutex> for thread-safe interior mutability
    let client = Arc::new(Mutex::new(client));

    // http.get(path, headers?)
    let client_clone = Arc::clone(&client);
    let get_fn = lua.create_function(move |lua, args: mlua::MultiValue| {
        let mut iter = args.into_iter();

        let path: String = match iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(mlua::Error::RuntimeError("http.get requires a path string".to_string())),
        };

        let headers = extract_headers(lua, iter.next())?;

        let client = client_clone.lock().map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
        match client.get(&path, headers) {
            Ok(response) => response_to_lua(lua, response),
            Err(e) => Err(mlua::Error::RuntimeError(format!("HTTP request failed: {}", e))),
        }
    })?;
    http.set("get", get_fn)?;

    // http.post(path, body?, headers?)
    let client_clone = Arc::clone(&client);
    let post_fn = lua.create_function(move |lua, args: mlua::MultiValue| {
        let mut iter = args.into_iter();

        let path: String = match iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(mlua::Error::RuntimeError("http.post requires a path string".to_string())),
        };

        let body = extract_body(lua, iter.next())?;
        let headers = extract_headers(lua, iter.next())?;

        let client = client_clone.lock().map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
        match client.post(&path, body, headers) {
            Ok(response) => response_to_lua(lua, response),
            Err(e) => Err(mlua::Error::RuntimeError(format!("HTTP request failed: {}", e))),
        }
    })?;
    http.set("post", post_fn)?;

    // http.put(path, body?, headers?)
    let client_clone = Arc::clone(&client);
    let put_fn = lua.create_function(move |lua, args: mlua::MultiValue| {
        let mut iter = args.into_iter();

        let path: String = match iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(mlua::Error::RuntimeError("http.put requires a path string".to_string())),
        };

        let body = extract_body(lua, iter.next())?;
        let headers = extract_headers(lua, iter.next())?;

        let client = client_clone.lock().map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
        match client.put(&path, body, headers) {
            Ok(response) => response_to_lua(lua, response),
            Err(e) => Err(mlua::Error::RuntimeError(format!("HTTP request failed: {}", e))),
        }
    })?;
    http.set("put", put_fn)?;

    // http.delete(path, headers?)
    let client_clone = Arc::clone(&client);
    let delete_fn = lua.create_function(move |lua, args: mlua::MultiValue| {
        let mut iter = args.into_iter();

        let path: String = match iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(mlua::Error::RuntimeError("http.delete requires a path string".to_string())),
        };

        let headers = extract_headers(lua, iter.next())?;

        let client = client_clone.lock().map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
        match client.delete(&path, headers) {
            Ok(response) => response_to_lua(lua, response),
            Err(e) => Err(mlua::Error::RuntimeError(format!("HTTP request failed: {}", e))),
        }
    })?;
    http.set("delete", delete_fn)?;

    // http.patch(path, body?, headers?)
    let client_clone = Arc::clone(&client);
    let patch_fn = lua.create_function(move |lua, args: mlua::MultiValue| {
        let mut iter = args.into_iter();

        let path: String = match iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => return Err(mlua::Error::RuntimeError("http.patch requires a path string".to_string())),
        };

        let body = extract_body(lua, iter.next())?;
        let headers = extract_headers(lua, iter.next())?;

        let client = client_clone.lock().map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
        match client.patch(&path, body, headers) {
            Ok(response) => response_to_lua(lua, response),
            Err(e) => Err(mlua::Error::RuntimeError(format!("HTTP request failed: {}", e))),
        }
    })?;
    http.set("patch", patch_fn)?;

    globals.set("http", http)?;
    Ok(())
}

/// Extract body from Lua value
fn extract_body(lua: &Lua, value: Option<Value>) -> LuaResult<Option<serde_json::Value>> {
    match value {
        Some(Value::Table(t)) => {
            let json = table_to_json(lua, &t)?;
            Ok(Some(json))
        }
        Some(Value::Nil) | None => Ok(None),
        _ => Ok(None),
    }
}

/// Extract headers from Lua value
fn extract_headers(_lua: &Lua, value: Option<Value>) -> LuaResult<Option<HashMap<String, String>>> {
    match value {
        Some(Value::Table(t)) => {
            let mut headers = HashMap::new();
            for pair in t.pairs::<String, String>() {
                let (k, v) = pair?;
                headers.insert(k, v);
            }
            Ok(Some(headers))
        }
        Some(Value::Nil) | None => Ok(None),
        _ => Ok(None),
    }
}

/// Convert Lua table to JSON
fn table_to_json(lua: &Lua, table: &Table) -> LuaResult<serde_json::Value> {
    // Check if it's an array
    let len = table.len()?;
    if len > 0 {
        let mut is_array = true;
        for i in 1..=len {
            if table.get::<Value>(i).is_err() {
                is_array = false;
                break;
            }
        }
        if is_array {
            let mut arr = Vec::new();
            for i in 1..=len {
                let val: Value = table.get(i)?;
                arr.push(value_to_json(lua, &val)?);
            }
            return Ok(serde_json::Value::Array(arr));
        }
    }

    // It's an object
    let mut map = serde_json::Map::new();
    for pair in table.pairs::<Value, Value>() {
        let (k, v) = pair?;
        let key = match k {
            Value::String(s) => s.to_str()?.to_string(),
            Value::Integer(i) => i.to_string(),
            _ => continue,
        };
        map.insert(key, value_to_json(lua, &v)?);
    }
    Ok(serde_json::Value::Object(map))
}

/// Convert Lua value to JSON
fn value_to_json(lua: &Lua, value: &Value) -> LuaResult<serde_json::Value> {
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
        Value::Integer(i) => Ok(serde_json::Value::Number((*i).into())),
        Value::Number(n) => {
            serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .ok_or_else(|| mlua::Error::RuntimeError("Invalid number".to_string()))
        }
        Value::String(s) => Ok(serde_json::Value::String(s.to_str()?.to_string())),
        Value::Table(t) => table_to_json(lua, t),
        _ => Ok(serde_json::Value::Null),
    }
}

/// Convert HTTP response to Lua table
fn response_to_lua(lua: &Lua, response: HttpResponse) -> LuaResult<Table> {
    let result = lua.create_table()?;

    // status
    result.set("status", response.status as i64)?;

    // body - convert JSON to Lua
    result.set("body", json_to_lua(lua, &response.body)?)?;

    // headers
    let headers_table = lua.create_table()?;
    for (k, v) in response.headers {
        headers_table.set(k, v)?;
    }
    result.set("headers", headers_table)?;

    // ok - convenience boolean
    result.set("ok", response.status >= 200 && response.status < 300)?;

    Ok(result)
}

/// Convert JSON to Lua value
fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> LuaResult<Value> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(obj) => {
            let table = lua.create_table()?;
            for (k, v) in obj {
                table.set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}
