//! HTTP client utilities for cluster operations.
//!
//! This module provides a shared HTTP client with connection pooling.
//! The client should be initialized at startup and shared across all handlers.

use std::sync::OnceLock;

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub fn init_http_client(client: reqwest::Client) {
    let _ = HTTP_CLIENT.set(client);
}

pub fn get_http_client() -> reqwest::Client {
    match HTTP_CLIENT.get() {
        Some(c) => c.clone(),
        None => reqwest::Client::new(),
    }
}

pub fn get_http_client_arc() -> reqwest::Client {
    HTTP_CLIENT.get_or_init(reqwest::Client::new).clone()
}
