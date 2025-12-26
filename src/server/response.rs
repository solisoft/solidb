use axum::{
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response, Json},
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFormat {
    Json,
    MsgPack,
}

impl ApiFormat {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        if let Some(accept) = headers.get(header::ACCEPT) {
            if let Ok(s) = accept.to_str() {
                // Check if client explicitly prefers msgpack
                // We could do full content negotiation parsing q-values but simple contains check is usually enough
                if s.contains("application/msgpack") || s.contains("application/x-msgpack") {
                    return ApiFormat::MsgPack;
                }
            }
        }
        ApiFormat::Json
    }
}

pub struct ApiResponse<T: Serialize> {
    pub data: T,
    pub format: ApiFormat,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn new(data: T, headers: &HeaderMap) -> Self {
        Self {
            data,
            format: ApiFormat::from_headers(headers),
        }
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        match self.format {
            ApiFormat::Json => Json(self.data).into_response(),
            ApiFormat::MsgPack => {
                // Use to_vec_named to serialize structs as maps (key-value) instead of arrays
                // This makes it compatible with JSON structure and most clients
                match rmp_serde::to_vec_named(&self.data) {
                    Ok(bytes) => {
                        (
                            [(header::CONTENT_TYPE, "application/msgpack")],
                            bytes
                        ).into_response()
                    }
                    Err(e) => {
                        // Fallback to JSON error if serialization fails
                        let err_body = serde_json::json!({
                            "error": format!("Serialization error: {}", e)
                        });
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(err_body)
                        ).into_response()
                    }
                }
            }
        }
    }
}
