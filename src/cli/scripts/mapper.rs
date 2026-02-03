//! Path mapping between file system and API paths
//!
//! Converts folder structure to API paths and parses Lua comment headers.

use std::path::Path;

/// Script metadata parsed from Lua comment header
#[derive(Debug, Clone, Default)]
pub struct ScriptMeta {
    /// HTTP methods this script handles
    pub methods: Vec<String>,
    /// Human-readable description
    pub description: Option<String>,
    /// Target collection (optional)
    pub collection: Option<String>,
}

impl ScriptMeta {
    /// Generate a name from the API path
    pub fn name_from_path(api_path: &str) -> String {
        api_path
            .trim_start_matches('/')
            .replace('/', "-")
            .replace(':', "")
    }
}

/// Parse Lua comment header to extract metadata
///
/// Supported directives:
/// - `@methods GET, POST, PUT` - HTTP methods (comma-separated)
/// - `@description User endpoint` - Human-readable description
/// - `@collection users` - Target collection
/// - `@ws` or `@ws true` - WebSocket endpoint
///
/// # Example
/// ```lua
/// -- @methods GET, POST
/// -- @description List or create users
/// -- @collection users
///
/// local users = db:collection("users")
/// ```
pub fn parse_script_meta(code: &str) -> ScriptMeta {
    let mut meta = ScriptMeta::default();

    for line in code.lines() {
        let line = line.trim();

        // Stop at first non-comment line
        if !line.starts_with("--") {
            break;
        }

        // Remove the comment prefix and trim
        let line = line.trim_start_matches("--").trim();

        if let Some(methods_str) = line.strip_prefix("@methods") {
            meta.methods = methods_str
                .trim()
                .split(',')
                .map(|m| m.trim().to_uppercase())
                .filter(|m| !m.is_empty())
                .collect();
        } else if let Some(desc) = line.strip_prefix("@description") {
            meta.description = Some(desc.trim().to_string());
        } else if let Some(coll) = line.strip_prefix("@collection") {
            meta.collection = Some(coll.trim().to_string());
        } else if line.starts_with("@ws") {
            // @ws or @ws true both mean WebSocket
            let value = line.strip_prefix("@ws").unwrap_or("").trim();
            if (value.is_empty() || value == "true") && !meta.methods.contains(&"WS".to_string()) {
                meta.methods.push("WS".to_string());
            }
        }
    }

    // Default to GET if no methods specified
    if meta.methods.is_empty() {
        meta.methods = vec!["GET".to_string()];
    }

    meta
}

/// Convert file path to API path
///
/// Rules:
/// - Filename (without .lua) = last segment of API path
/// - `_paramname.lua` or `_paramname/` → path parameter (`:paramname`)
/// - Folder structure = API path hierarchy
///
/// # Examples
/// - `hello.lua` → `/hello`
/// - `users.lua` → `/users`
/// - `users/_id.lua` → `/users/:id`
/// - `api/v1/products.lua` → `/api/v1/products`
pub fn file_to_api_path(file_path: &Path, base: &Path) -> String {
    let rel_path = match file_path.strip_prefix(base) {
        Ok(p) => p,
        Err(_) => file_path,
    };

    let mut segments = Vec::new();

    // Process directory components
    if let Some(parent) = rel_path.parent() {
        for component in parent.components() {
            if let std::path::Component::Normal(seg) = component {
                if let Some(seg_str) = seg.to_str() {
                    if let Some(param) = seg_str.strip_prefix('_') {
                        segments.push(format!(":{}", param));
                    } else {
                        segments.push(seg_str.to_string());
                    }
                }
            }
        }
    }

    // Process filename (without .lua extension)
    if let Some(stem) = rel_path.file_stem() {
        if let Some(name) = stem.to_str() {
            if let Some(param) = name.strip_prefix('_') {
                segments.push(format!(":{}", param));
            } else {
                segments.push(name.to_string());
            }
        }
    }

    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

/// Convert API path to file path
///
/// Inverse of `file_to_api_path`:
/// - `/users/:id` → `users/_id.lua`
/// - `/api/v1/products` → `api/v1/products.lua`
pub fn api_path_to_file(api_path: &str, base: &Path) -> std::path::PathBuf {
    let segments: Vec<String> = api_path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|seg| {
            if let Some(param) = seg.strip_prefix(':') {
                // :id → _id
                format!("_{}", param)
            } else {
                seg.to_string()
            }
        })
        .collect();

    let mut path = base.to_path_buf();

    if segments.is_empty() {
        path.push("index.lua");
    } else {
        // All segments except last become directories
        for seg in &segments[..segments.len().saturating_sub(1)] {
            path.push(seg);
        }

        // Last segment becomes filename
        if let Some(last) = segments.last() {
            path.push(format!("{}.lua", last));
        }
    }

    path
}

/// Generate comment header for a script
///
/// Creates Lua comment lines from script metadata.
pub fn generate_header(
    methods: &[String],
    description: Option<&str>,
    collection: Option<&str>,
) -> String {
    let mut header = String::new();

    // Methods
    header.push_str(&format!("-- @methods {}\n", methods.join(", ")));

    // Description
    if let Some(desc) = description {
        header.push_str(&format!("-- @description {}\n", desc));
    }

    // Collection
    if let Some(coll) = collection {
        header.push_str(&format!("-- @collection {}\n", coll));
    }

    header.push('\n');
    header
}

/// Check if the code already has a metadata header
pub fn has_metadata_header(code: &str) -> bool {
    for line in code.lines() {
        let line = line.trim();
        if !line.starts_with("--") {
            return false;
        }
        if line.contains("@methods")
            || line.contains("@description")
            || line.contains("@collection")
            || line.contains("@ws")
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_script_meta_basic() {
        let code = r#"-- @methods GET, POST
-- @description User management
-- @collection users

local users = db:collection("users")
"#;
        let meta = parse_script_meta(code);
        assert_eq!(meta.methods, vec!["GET", "POST"]);
        assert_eq!(meta.description, Some("User management".to_string()));
        assert_eq!(meta.collection, Some("users".to_string()));
    }

    #[test]
    fn test_parse_script_meta_ws() {
        let code = r#"-- @methods GET
-- @ws

return "hello"
"#;
        let meta = parse_script_meta(code);
        assert_eq!(meta.methods, vec!["GET", "WS"]);
    }

    #[test]
    fn test_parse_script_meta_default() {
        let code = r#"-- This is a simple script
return "hello"
"#;
        let meta = parse_script_meta(code);
        assert_eq!(meta.methods, vec!["GET"]);
    }

    #[test]
    fn test_file_to_api_path() {
        let base = PathBuf::from("/scripts");

        assert_eq!(
            file_to_api_path(&PathBuf::from("/scripts/hello.lua"), &base),
            "/hello"
        );
        assert_eq!(
            file_to_api_path(&PathBuf::from("/scripts/users.lua"), &base),
            "/users"
        );
        assert_eq!(
            file_to_api_path(&PathBuf::from("/scripts/users/_id.lua"), &base),
            "/users/:id"
        );
        assert_eq!(
            file_to_api_path(&PathBuf::from("/scripts/api/v1/products.lua"), &base),
            "/api/v1/products"
        );
    }

    #[test]
    fn test_api_path_to_file() {
        let base = PathBuf::from("/scripts");

        assert_eq!(
            api_path_to_file("/hello", &base),
            PathBuf::from("/scripts/hello.lua")
        );
        assert_eq!(
            api_path_to_file("/users/:id", &base),
            PathBuf::from("/scripts/users/_id.lua")
        );
        assert_eq!(
            api_path_to_file("/api/v1/products", &base),
            PathBuf::from("/scripts/api/v1/products.lua")
        );
    }

    #[test]
    fn test_roundtrip() {
        let base = PathBuf::from("/scripts");
        let paths = vec!["/hello", "/users/:id", "/api/v1/products"];

        for path in paths {
            let file = api_path_to_file(path, &base);
            let back = file_to_api_path(&file, &base);
            assert_eq!(path, back);
        }
    }

    #[test]
    fn test_generate_header() {
        let header = generate_header(
            &["GET".to_string(), "POST".to_string()],
            Some("User endpoint"),
            Some("users"),
        );
        assert!(header.contains("-- @methods GET, POST"));
        assert!(header.contains("-- @description User endpoint"));
        assert!(header.contains("-- @collection users"));
    }
}
