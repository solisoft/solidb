//! What a script hands back to the HTTP layer.
//!
//! A script's return value used to be serialised as a JSON 200 no matter
//! what; `response.html(...)` produced a marker string nothing interpreted,
//! `solidb.error("...", 404)` came out as a 500, and the tutorials'
//! `solidb.status(201)` did not exist. This module is the one contract:
//!
//! * a plain Lua value is still a JSON body, status 200, unless the script
//!   set `solidb.status(code)` / `solidb.header(name, value)` first;
//! * `response.json / html / redirect / file / cors` return an opaque
//!   [`ScriptResponse`] carrying status, headers and body;
//! * `solidb.error(msg, code)` raises, and the engine turns it into that
//!   HTTP status (`crate::error::DbError::ScriptError`).
//!
//! The per-request overrides live in mlua app data, like the caller
//! identity: a script cannot reach or clobber them, and a pooled state gets
//! a fresh set on every request.

use mlua::{Function, Lua, Table, UserData, Value as LuaValue};
use serde_json::Value as JsonValue;

use crate::scripting::conversion::lua_to_json_value;

/// The body of a script response.
#[derive(Debug, Clone)]
pub enum ResponseBody {
    Json(JsonValue),
    Raw {
        content_type: String,
        bytes: Vec<u8>,
    },
    /// A stored file (`solidb.upload` / the `_files` collection), resolved
    /// by the engine, which knows the database.
    File {
        key: String,
        filename: Option<String>,
    },
}

/// An explicit response built with `response.*`.
#[derive(Debug, Clone)]
pub struct ScriptResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: ResponseBody,
}

impl UserData for ScriptResponse {}

/// Status and headers set by `solidb.status` / `solidb.header` before the
/// script returned a plain value.
#[derive(Debug, Clone, Default)]
pub struct ResponseOverrides {
    pub status: Option<u16>,
    pub headers: Vec<(String, String)>,
}

/// Start a request with no overrides. Called from the same places the
/// caller identity is installed.
pub fn reset_overrides(lua: &Lua) {
    lua.set_app_data(ResponseOverrides::default());
}

/// The overrides the script left behind, and a clean slate for the next one.
pub fn take_overrides(lua: &Lua) -> ResponseOverrides {
    lua.remove_app_data::<ResponseOverrides>()
        .unwrap_or_default()
}

fn valid_status(code: u16) -> Result<u16, mlua::Error> {
    if (100..=599).contains(&code) {
        Ok(code)
    } else {
        Err(mlua::Error::RuntimeError(format!(
            "invalid HTTP status {}",
            code
        )))
    }
}

fn headers_from(lua: &Lua, value: Option<LuaValue>) -> Result<Vec<(String, String)>, mlua::Error> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let LuaValue::Table(t) = value else {
        return Err(mlua::Error::RuntimeError(
            "headers must be a table of name = value".to_string(),
        ));
    };
    let mut out = Vec::new();
    for pair in t.pairs::<String, LuaValue>() {
        let (k, v) = pair?;
        let v = match v {
            LuaValue::String(s) => s.to_str()?.to_string(),
            other => lua_to_json_value(lua, other)?.to_string(),
        };
        out.push((k, v));
    }
    Ok(out)
}

/// `solidb.status(code)`: the status a plain return value is sent with.
pub fn create_status_function(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(|lua, code: u16| {
        let code = valid_status(code)?;
        let mut ov = lua
            .app_data_mut::<ResponseOverrides>()
            .ok_or_else(|| mlua::Error::RuntimeError("no request in progress".to_string()))?;
        ov.status = Some(code);
        Ok(())
    })
}

/// `solidb.header(name, value)`: a header added to a plain return value.
pub fn create_header_function(lua: &Lua) -> mlua::Result<Function> {
    lua.create_function(|lua, (name, value): (String, String)| {
        let mut ov = lua
            .app_data_mut::<ResponseOverrides>()
            .ok_or_else(|| mlua::Error::RuntimeError("no request in progress".to_string()))?;
        ov.headers.push((name, value));
        Ok(())
    })
}

/// The `response` global.
pub fn create_response_table(lua: &Lua) -> mlua::Result<Table> {
    let response = lua.create_table()?;

    // response.json(data, status?, headers?)
    response.set(
        "json",
        lua.create_function(
            |lua, (data, status, headers): (LuaValue, Option<u16>, Option<LuaValue>)| {
                let body = lua_to_json_value(lua, data)?;
                lua.create_userdata(ScriptResponse {
                    status: status.map(valid_status).transpose()?.unwrap_or(200),
                    headers: headers_from(lua, headers)?,
                    body: ResponseBody::Json(body),
                })
            },
        )?,
    )?;

    // response.html(content, status?)
    response.set(
        "html",
        lua.create_function(|lua, (content, status): (String, Option<u16>)| {
            lua.create_userdata(ScriptResponse {
                status: status.map(valid_status).transpose()?.unwrap_or(200),
                headers: Vec::new(),
                body: ResponseBody::Raw {
                    content_type: "text/html; charset=utf-8".to_string(),
                    bytes: content.into_bytes(),
                },
            })
        })?,
    )?;

    // response.redirect(url, status?) — 302 unless told otherwise
    response.set(
        "redirect",
        lua.create_function(|lua, (url, status): (String, Option<u16>)| {
            lua.create_userdata(ScriptResponse {
                status: status.map(valid_status).transpose()?.unwrap_or(302),
                headers: vec![("Location".to_string(), url)],
                body: ResponseBody::Raw {
                    content_type: "text/plain; charset=utf-8".to_string(),
                    bytes: Vec::new(),
                },
            })
        })?,
    )?;

    // response.file(key, filename?) — a file stored with solidb.upload
    response.set(
        "file",
        lua.create_function(|lua, (key, filename): (String, Option<String>)| {
            lua.create_userdata(ScriptResponse {
                status: 200,
                headers: Vec::new(),
                body: ResponseBody::File { key, filename },
            })
        })?,
    )?;

    // response.cors(data, options?) — a JSON body with CORS headers.
    // options: { origin = "*", methods = "GET, POST", headers = "...",
    //            credentials = true, max_age = 600 }
    response.set(
        "cors",
        lua.create_function(|lua, (data, options): (LuaValue, Option<Table>)| {
            let body = lua_to_json_value(lua, data)?;
            let get = |k: &str| -> Result<Option<String>, mlua::Error> {
                match &options {
                    Some(t) => match t.get::<LuaValue>(k)? {
                        LuaValue::Nil => Ok(None),
                        LuaValue::String(s) => Ok(Some(s.to_str()?.to_string())),
                        LuaValue::Boolean(b) => Ok(Some(b.to_string())),
                        LuaValue::Integer(i) => Ok(Some(i.to_string())),
                        other => Ok(Some(lua_to_json_value(lua, other)?.to_string())),
                    },
                    None => Ok(None),
                }
            };
            let mut headers = vec![(
                "Access-Control-Allow-Origin".to_string(),
                get("origin")?.unwrap_or_else(|| "*".to_string()),
            )];
            headers.push((
                "Access-Control-Allow-Methods".to_string(),
                get("methods")?.unwrap_or_else(|| "GET, POST, PUT, DELETE, OPTIONS".to_string()),
            ));
            if let Some(h) = get("headers")? {
                headers.push(("Access-Control-Allow-Headers".to_string(), h));
            }
            if get("credentials")?.as_deref() == Some("true") {
                headers.push((
                    "Access-Control-Allow-Credentials".to_string(),
                    "true".to_string(),
                ));
            }
            if let Some(age) = get("max_age")? {
                headers.push(("Access-Control-Max-Age".to_string(), age));
            }
            lua.create_userdata(ScriptResponse {
                status: 200,
                headers,
                body: ResponseBody::Json(body),
            })
        })?,
    )?;

    Ok(response)
}
