//! LLM Client for Natural Language to SDBQL translation
//!
//! Supports OpenAI, Anthropic, and Ollama providers.
//! Reads credentials from _system database's _env collection.

use crate::error::DbError;
use crate::storage::StorageEngine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Supported LLM providers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LLMProvider {
    OpenAI,
    Anthropic,
    Ollama,
    Gemini,
}

impl LLMProvider {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self, DbError> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(LLMProvider::OpenAI),
            "anthropic" => Ok(LLMProvider::Anthropic),
            "ollama" => Ok(LLMProvider::Ollama),
            "gemini" => Ok(LLMProvider::Gemini),
            _ => Err(DbError::ExecutionError(format!(
                "Unknown LLM provider: {}. Supported: openai, anthropic, ollama, gemini",
                s
            ))),
        }
    }
}

/// Configuration for LLM client
#[derive(Debug, Clone)]
pub struct LLMConfig {
    pub provider: LLMProvider,
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    /// Optional embedding-specific model (falls back to chat model or provider default)
    pub embedding_model: Option<String>,
}

/// Message in a chat conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: &str) -> Self {
        Message {
            role: "system".to_string(),
            content: content.to_string(),
        }
    }

    pub fn user(content: &str) -> Self {
        Message {
            role: "user".to_string(),
            content: content.to_string(),
        }
    }

    pub fn assistant(content: &str) -> Self {
        Message {
            role: "assistant".to_string(),
            content: content.to_string(),
        }
    }
}

/// LLM client for making chat completions
#[derive(Clone)]
pub struct LLMClient {
    config: LLMConfig,
    http_client: Client,
}

/// Helper to get env var from database _env collection or OS environment
/// Checks current database first, then _system, then OS environment
fn get_env_var(storage: &StorageEngine, db_name: &str, key: &str) -> Option<String> {
    // First, try the current database's _env collection
    if let Ok(db) = storage.get_database(db_name) {
        if let Ok(coll) = db.system_collection("_env") {
            if let Ok(doc) = coll.get(key) {
                if let Some(value) = doc.get("value") {
                    if let Some(s) = value.as_str() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    // Then try _system database's _env collection
    if db_name != "_system" {
        if let Ok(db) = storage.get_database("_system") {
            if let Ok(coll) = db.system_collection("_env") {
                if let Ok(doc) = coll.get(key) {
                    if let Some(value) = doc.get("value") {
                        if let Some(s) = value.as_str() {
                            return Some(s.to_string());
                        }
                    }
                }
            }
        }
    }
    // Fallback to OS environment variable
    std::env::var(key).ok()
}

impl LLMClient {
    /// Create LLM client from database _env collection
    ///
    /// Reads credentials based on provider:
    /// - OpenAI: OPENAI_API_KEY, OPENAI_MODEL (default: gpt-4o)
    /// - Anthropic: ANTHROPIC_API_KEY, ANTHROPIC_MODEL (default: claude-sonnet-4-20250514)
    /// - Ollama: OLLAMA_URL (default: http://localhost:11434), OLLAMA_MODEL (default: llama3)
    /// - Gemini: GEMINI_API_KEY, GEMINI_MODEL (default: gemini-1.5-pro)
    ///
    /// Checks current database _env first, then _system/_env, then OS environment.
    /// Default provider from NL_DEFAULT_PROVIDER (default: anthropic) — kept for
    /// backward compatibility with existing chat/NL deployments. Embedding callers
    /// pass an explicit provider (embeddings default to openai) plus embedding_model
    /// from the vector index config to honor the per-index model.
    pub fn from_storage(
        storage: &StorageEngine,
        db_name: &str,
        provider: Option<&str>,
        embedding_model: Option<String>,
    ) -> Result<Self, DbError> {
        let provider_str = provider
            .map(|s| s.to_string())
            .or_else(|| get_env_var(storage, db_name, "NL_DEFAULT_PROVIDER"))
            .unwrap_or_else(|| "anthropic".to_string());

        let provider = LLMProvider::from_str(&provider_str)?;

        let embedding_model = embedding_model.or_else(|| match provider {
            LLMProvider::OpenAI => get_env_var(storage, db_name, "OPENAI_EMBEDDING_MODEL"),
            LLMProvider::Ollama => get_env_var(storage, db_name, "OLLAMA_EMBEDDING_MODEL"),
            LLMProvider::Gemini => get_env_var(storage, db_name, "GEMINI_EMBEDDING_MODEL"),
            LLMProvider::Anthropic => None,
        });

        let config = match provider {
            LLMProvider::OpenAI => {
                let api_key = get_env_var(storage, db_name, "OPENAI_API_KEY").ok_or_else(|| {
                    DbError::ExecutionError(
                        "OPENAI_API_KEY not found in _env collection".to_string(),
                    )
                })?;
                let model = get_env_var(storage, db_name, "OPENAI_MODEL")
                    .unwrap_or_else(|| "gpt-4o".to_string());
                LLMConfig {
                    provider,
                    api_url: "https://api.openai.com/v1/chat/completions".to_string(),
                    api_key,
                    model,
                    embedding_model,
                }
            }
            LLMProvider::Anthropic => {
                let api_key =
                    get_env_var(storage, db_name, "ANTHROPIC_API_KEY").ok_or_else(|| {
                        DbError::ExecutionError(
                            "ANTHROPIC_API_KEY not found in _env collection".to_string(),
                        )
                    })?;
                let model = get_env_var(storage, db_name, "ANTHROPIC_MODEL")
                    .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
                LLMConfig {
                    provider,
                    api_url: "https://api.anthropic.com/v1/messages".to_string(),
                    api_key,
                    model,
                    embedding_model,
                }
            }
            LLMProvider::Ollama => {
                let base_url = get_env_var(storage, db_name, "OLLAMA_URL")
                    .unwrap_or_else(|| "http://localhost:11434".to_string());
                // Trim whitespace and trailing slashes to avoid URL issues
                let base_url = base_url.trim().trim_end_matches('/');
                // Ensure URL has http:// scheme (Ollama is always HTTP locally)
                let base_url =
                    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
                        format!("http://{}", base_url)
                    } else {
                        base_url.to_string()
                    };
                let model = get_env_var(storage, db_name, "OLLAMA_MODEL")
                    .unwrap_or_else(|| "llama3".to_string())
                    .trim()
                    .to_string();
                LLMConfig {
                    provider,
                    api_url: format!("{}/api/chat", base_url),
                    api_key: String::new(), // Ollama doesn't need API key
                    model,
                    embedding_model,
                }
            }
            LLMProvider::Gemini => {
                let api_key = get_env_var(storage, db_name, "GEMINI_API_KEY").ok_or_else(|| {
                    DbError::ExecutionError(
                        "GEMINI_API_KEY not found in _env collection".to_string(),
                    )
                })?;
                let model = get_env_var(storage, db_name, "GEMINI_MODEL")
                    .unwrap_or_else(|| "gemini-1.5-pro".to_string());
                LLMConfig {
                    provider,
                    // URL will be constructed dynamically in chat_gemini to include model
                    api_url: "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
                    api_key,
                    model,
                    embedding_model,
                }
            }
        };

        Ok(LLMClient {
            config,
            // No redirects: the only tenant-controllable base URL is the
            // Ollama one, and a redirect would take the request (and the
            // instance-wide API key it carries) somewhere the SSRF guard
            // never saw.
            http_client: Client::builder()
                .timeout(Duration::from_secs(120))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| Client::new()),
        })
    }

    /// Create an LLM client using only process environment variables (no DB _env lookup).
    /// Useful for auto-embedding in storage layer where full StorageEngine may be inconvenient.
    pub fn from_env(
        provider: Option<&str>,
        embedding_model: Option<String>,
    ) -> Result<Self, DbError> {
        let provider_str = provider
            .map(|s| s.to_string())
            .or_else(|| std::env::var("NL_DEFAULT_PROVIDER").ok())
            .unwrap_or_else(|| "openai".to_string()); // prefer openai for embeddings

        let provider = LLMProvider::from_str(&provider_str)?;

        let config = match provider {
            LLMProvider::OpenAI => {
                let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                    DbError::ExecutionError("OPENAI_API_KEY not found in environment".to_string())
                })?;
                let model = embedding_model.unwrap_or_else(|| {
                    std::env::var("OPENAI_EMBEDDING_MODEL")
                        .unwrap_or_else(|_| "text-embedding-3-small".to_string())
                });
                LLMConfig {
                    provider,
                    api_url: "https://api.openai.com/v1/chat/completions".to_string(),
                    api_key,
                    model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".to_string()),
                    embedding_model: Some(model),
                }
            }
            LLMProvider::Ollama => {
                let base_url = std::env::var("OLLAMA_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());
                let base_url = base_url.trim().trim_end_matches('/').to_string();
                let base_url = if !base_url.starts_with("http") {
                    format!("http://{}", base_url)
                } else {
                    base_url
                };
                let model = embedding_model.unwrap_or_else(|| {
                    std::env::var("OLLAMA_EMBEDDING_MODEL")
                        .unwrap_or_else(|_| "nomic-embed-text".to_string())
                });
                LLMConfig {
                    provider,
                    api_url: format!("{}/api/chat", base_url),
                    api_key: String::new(),
                    model: std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3".to_string()),
                    embedding_model: Some(model),
                }
            }
            LLMProvider::Gemini => {
                let api_key = std::env::var("GEMINI_API_KEY").map_err(|_| {
                    DbError::ExecutionError("GEMINI_API_KEY not found in environment".to_string())
                })?;
                let model = embedding_model.unwrap_or_else(|| {
                    std::env::var("GEMINI_EMBEDDING_MODEL")
                        .unwrap_or_else(|_| "text-embedding-004".to_string())
                });
                LLMConfig {
                    provider,
                    api_url: "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
                    api_key,
                    model: std::env::var("GEMINI_MODEL")
                        .unwrap_or_else(|_| "gemini-1.5-pro".to_string()),
                    embedding_model: Some(model),
                }
            }
            LLMProvider::Anthropic => {
                return Err(DbError::ExecutionError(
                    "Anthropic embeddings not supported via env-only client".into(),
                ));
            }
        };

        Ok(LLMClient {
            config,
            // No redirects: the only tenant-controllable base URL is the
            // Ollama one, and a redirect would take the request (and the
            // instance-wide API key it carries) somewhere the SSRF guard
            // never saw.
            http_client: Client::builder()
                .timeout(Duration::from_secs(120))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| Client::new()),
        })
    }

    /// Get the configured provider
    pub fn provider(&self) -> LLMProvider {
        self.config.provider
    }

    /// Send chat messages and get response
    pub async fn chat(&self, messages: Vec<Message>) -> Result<String, DbError> {
        match self.config.provider {
            LLMProvider::OpenAI => self.chat_openai(messages).await,
            LLMProvider::Anthropic => self.chat_anthropic(messages).await,
            LLMProvider::Ollama => self.chat_ollama(messages).await,
            LLMProvider::Gemini => self.chat_gemini(messages).await,
        }
    }

    /// Generate an embedding vector for the given text.
    /// Uses the configured provider's embedding endpoint.
    /// Supports dimension reduction via model choice where applicable.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, DbError> {
        // Use override model if set on config (we store it in LLMConfig? extend lightly)
        let model = self.config.embedding_model.clone().unwrap_or_else(|| {
            match self.config.provider {
                LLMProvider::OpenAI => "text-embedding-3-small".to_string(),
                LLMProvider::Ollama => "nomic-embed-text".to_string(),
                LLMProvider::Gemini => "text-embedding-004".to_string(),
                LLMProvider::Anthropic => "claude-3-haiku".to_string(), // not native embed; will error
            }
        });

        match self.config.provider {
            LLMProvider::OpenAI => self.embed_openai(text, &model).await,
            LLMProvider::Ollama => self.embed_ollama(text, &model).await,
            LLMProvider::Gemini => self.embed_gemini(text, &model).await,
            LLMProvider::Anthropic => Err(DbError::ExecutionError(
                "Anthropic does not provide native embeddings in this client. Use OpenAI, Ollama or Gemini for auto-embeddings.".to_string(),
            )),
        }
    }

    /// Blocking variant useful from sync collection paths or backfills.
    /// Uses a dedicated OS thread + fresh runtime to be safe on any tokio runtime
    /// (including current-thread used by some replication/Lua paths).
    pub fn embed_blocking(&self, text: &str) -> Result<Vec<f32>, DbError> {
        let this = self.clone();
        let text = text.to_owned();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().map_err(|e| {
                DbError::ExecutionError(format!("Failed to create runtime for embedding: {}", e))
            })?;
            rt.block_on(this.embed(&text))
        })
        .join()
        .map_err(|_| DbError::ExecutionError("embedding thread panicked".to_string()))?
    }

    /// Blocking chat completion — spawns a dedicated OS thread + fresh runtime so
    /// it's safe to call from synchronous contexts such as the SDBQL executor
    /// (mirrors `embed_blocking`). Used by the `RERANK` function's LLM mode.
    pub fn chat_blocking(&self, messages: Vec<Message>) -> Result<String, DbError> {
        let this = self.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().map_err(|e| {
                DbError::ExecutionError(format!("Failed to create runtime for chat: {}", e))
            })?;
            rt.block_on(this.chat(messages))
        })
        .join()
        .map_err(|_| DbError::ExecutionError("chat thread panicked".to_string()))?
    }

    /// Batch embedding for efficiency during index builds on large collections.
    /// OpenAI supports native batch; others fall back to sequential inside one blocking call.
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, DbError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        match self.config.provider {
            LLMProvider::OpenAI => self.embed_batch_openai(texts).await,
            _ => {
                // sequential fallback
                let mut results = Vec::with_capacity(texts.len());
                for t in texts {
                    results.push(self.embed(t).await?);
                }
                Ok(results)
            }
        }
    }

    /// Blocking batch variant.
    pub fn embed_batch_blocking(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, DbError> {
        let this = self.clone();
        let texts: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().map_err(|e| {
                DbError::ExecutionError(format!(
                    "Failed to create runtime for batch embedding: {}",
                    e
                ))
            })?;
            let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            rt.block_on(this.embed_batch(&refs))
        })
        .join()
        .map_err(|_| DbError::ExecutionError("batch embedding thread panicked".to_string()))?
    }

    async fn chat_openai(&self, messages: Vec<Message>) -> Result<String, DbError> {
        #[derive(Serialize)]
        struct OpenAIRequest {
            model: String,
            messages: Vec<Message>,
            temperature: f32,
        }

        #[derive(Deserialize)]
        struct OpenAIResponse {
            choices: Vec<OpenAIChoice>,
        }

        #[derive(Deserialize)]
        struct OpenAIChoice {
            message: OpenAIMessage,
        }

        #[derive(Deserialize)]
        struct OpenAIMessage {
            content: String,
        }

        let request = OpenAIRequest {
            model: self.config.model.clone(),
            messages,
            temperature: 0.0,
        };

        let response = self
            .http_client
            .post(&self.config.api_url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| DbError::ExecutionError(format!("OpenAI API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DbError::ExecutionError(format!(
                "OpenAI API error {}: {}",
                status, body
            )));
        }

        let result: OpenAIResponse = response.json().await.map_err(|e| {
            DbError::ExecutionError(format!("Failed to parse OpenAI response: {}", e))
        })?;

        result
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .ok_or_else(|| DbError::ExecutionError("No response from OpenAI".to_string()))
    }

    async fn chat_anthropic(&self, messages: Vec<Message>) -> Result<String, DbError> {
        #[derive(Serialize)]
        struct AnthropicRequest {
            model: String,
            max_tokens: u32,
            system: Option<String>,
            messages: Vec<AnthropicMessage>,
        }

        #[derive(Serialize)]
        struct AnthropicMessage {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct AnthropicResponse {
            content: Vec<AnthropicContent>,
        }

        #[derive(Deserialize)]
        struct AnthropicContent {
            text: String,
        }

        // Extract system message and convert others
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.clone());

        let api_messages: Vec<AnthropicMessage> = messages
            .into_iter()
            .filter(|m| m.role != "system")
            .map(|m| AnthropicMessage {
                role: m.role,
                content: m.content,
            })
            .collect();

        let request = AnthropicRequest {
            model: self.config.model.clone(),
            max_tokens: 1024,
            system,
            messages: api_messages,
        };

        let response = self
            .http_client
            .post(&self.config.api_url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| DbError::ExecutionError(format!("Anthropic API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DbError::ExecutionError(format!(
                "Anthropic API error {}: {}",
                status, body
            )));
        }

        let result: AnthropicResponse = response.json().await.map_err(|e| {
            DbError::ExecutionError(format!("Failed to parse Anthropic response: {}", e))
        })?;

        result
            .content
            .first()
            .map(|c| c.text.trim().to_string())
            .ok_or_else(|| DbError::ExecutionError("No response from Anthropic".to_string()))
    }

    async fn chat_ollama(&self, messages: Vec<Message>) -> Result<String, DbError> {
        #[derive(Serialize)]
        struct OllamaRequest {
            model: String,
            messages: Vec<Message>,
            stream: bool,
        }

        #[derive(Deserialize)]
        struct OllamaResponse {
            message: OllamaMessage,
        }

        #[derive(Deserialize)]
        struct OllamaMessage {
            content: String,
        }

        let request = OllamaRequest {
            model: self.config.model.clone(),
            messages,
            stream: false,
        };

        let response = self
            .http_client
            .post(&self.config.api_url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                DbError::ExecutionError(format!(
                    "Ollama API request to '{}' failed: {}",
                    self.config.api_url, e
                ))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DbError::ExecutionError(format!(
                "Ollama API error {}: {}",
                status, body
            )));
        }

        let result: OllamaResponse = response.json().await.map_err(|e| {
            DbError::ExecutionError(format!("Failed to parse Ollama response: {}", e))
        })?;

        Ok(result.message.content.trim().to_string())
    }

    async fn chat_gemini(&self, messages: Vec<Message>) -> Result<String, DbError> {
        #[derive(Serialize)]
        struct GeminiRequest {
            contents: Vec<GeminiContent>,
            #[serde(skip_serializing_if = "Option::is_none")]
            system_instruction: Option<GeminiSystem>,
        }

        #[derive(Serialize)]
        struct GeminiContent {
            role: String,
            parts: Vec<GeminiPart>,
        }

        #[derive(Serialize)]
        struct GeminiSystem {
            parts: Vec<GeminiPart>,
        }

        #[derive(Serialize)]
        struct GeminiPart {
            text: String,
        }

        #[derive(Deserialize)]
        struct GeminiResponse {
            candidates: Option<Vec<GeminiCandidate>>,
        }

        #[derive(Deserialize)]
        struct GeminiCandidate {
            content: Option<GeminiContentResponse>,
        }

        #[derive(Deserialize)]
        struct GeminiContentResponse {
            parts: Option<Vec<GeminiPartResponse>>,
        }

        #[derive(Deserialize)]
        struct GeminiPartResponse {
            text: Option<String>,
        }

        // Extract system message
        let system_instruction =
            messages
                .iter()
                .find(|m| m.role == "system")
                .map(|m| GeminiSystem {
                    parts: vec![GeminiPart {
                        text: m.content.clone(),
                    }],
                });

        // Convert messages (skip system as it's handled separately)
        let contents: Vec<GeminiContent> = messages
            .into_iter()
            .filter(|m| m.role != "system")
            .map(|m| {
                let role = if m.role == "assistant" {
                    "model".to_string()
                } else {
                    "user".to_string()
                };
                GeminiContent {
                    role,
                    parts: vec![GeminiPart { text: m.content }],
                }
            })
            .collect();

        let request = GeminiRequest {
            contents,
            system_instruction,
        };

        // Key in a header, not the query string. reqwest's transport errors
        // stringify as `... for url (<the full url>)`, and those errors are
        // returned to the caller and logged by the embedding worker — so with
        // `?key=` a single provider timeout printed the instance-wide Gemini
        // credential into the logs and into an ordinary API response.
        let url = format!(
            "{}/{}:generateContent",
            self.config.api_url, self.config.model
        );

        let response = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &self.config.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| DbError::ExecutionError(format!("Gemini API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DbError::ExecutionError(format!(
                "Gemini API error {}: {}",
                status, body
            )));
        }

        let result: GeminiResponse = response.json().await.map_err(|e| {
            DbError::ExecutionError(format!("Failed to parse Gemini response: {}", e))
        })?;

        result
            .candidates
            .and_then(|c| c.into_iter().next())
            .and_then(|c| c.content)
            .and_then(|c| c.parts)
            .and_then(|p| p.into_iter().next())
            .and_then(|p| p.text)
            .map(|t| t.trim().to_string())
            .ok_or_else(|| DbError::ExecutionError("No response content from Gemini".to_string()))
    }

    // ==================== Embedding implementations ====================

    async fn embed_openai(&self, text: &str, model: &str) -> Result<Vec<f32>, DbError> {
        #[derive(Serialize)]
        struct EmbedRequest {
            model: String,
            input: String,
        }

        #[derive(Deserialize)]
        struct EmbedResponse {
            data: Vec<EmbedData>,
        }

        #[derive(Deserialize)]
        struct EmbedData {
            embedding: Vec<f32>,
        }

        // Derive embeddings endpoint from chat api_url if possible (supports custom gateways),
        // otherwise fall back to public OpenAI.
        let url = if self.config.api_url.contains("/chat/completions") {
            self.config
                .api_url
                .replace("/chat/completions", "/embeddings")
        } else if self.config.api_url.contains("openai.com") {
            "https://api.openai.com/v1/embeddings".to_string()
        } else {
            // Assume the configured url is base, append embeddings path
            format!("{}/embeddings", self.config.api_url.trim_end_matches('/'))
        };

        let request = EmbedRequest {
            model: model.to_string(),
            input: text.to_string(),
        };

        let response = self
            .http_client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                DbError::ExecutionError(format!("OpenAI embeddings request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DbError::ExecutionError(format!(
                "OpenAI embeddings error {}: {}",
                status, body
            )));
        }

        let result: EmbedResponse = response.json().await.map_err(|e| {
            DbError::ExecutionError(format!("Failed to parse OpenAI embeddings response: {}", e))
        })?;

        result
            .data
            .first()
            .map(|d| d.embedding.clone())
            .ok_or_else(|| DbError::ExecutionError("No embedding returned from OpenAI".to_string()))
    }

    async fn embed_batch_openai(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, DbError> {
        #[derive(Serialize)]
        struct EmbedBatchRequest {
            model: String,
            input: Vec<String>,
        }

        #[derive(Deserialize)]
        struct EmbedBatchResponse {
            data: Vec<EmbedData>,
        }

        #[derive(Deserialize)]
        struct EmbedData {
            embedding: Vec<f32>,
        }

        let url = if self.config.api_url.contains("/chat/completions") {
            self.config
                .api_url
                .replace("/chat/completions", "/embeddings")
        } else if self.config.api_url.contains("openai.com") {
            "https://api.openai.com/v1/embeddings".to_string()
        } else {
            format!("{}/embeddings", self.config.api_url.trim_end_matches('/'))
        };

        let input: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        let request = EmbedBatchRequest {
            model: self
                .config
                .embedding_model
                .clone()
                .unwrap_or_else(|| "text-embedding-3-small".to_string()),
            input,
        };

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                DbError::ExecutionError(format!("OpenAI batch embeddings request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DbError::ExecutionError(format!(
                "OpenAI batch embeddings error {}: {}",
                status, body
            )));
        }

        let result: EmbedBatchResponse = response.json().await.map_err(|e| {
            DbError::ExecutionError(format!(
                "Failed to parse OpenAI batch embeddings response: {}",
                e
            ))
        })?;

        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    async fn embed_ollama(&self, text: &str, model: &str) -> Result<Vec<f32>, DbError> {
        #[derive(Serialize)]
        struct OllamaEmbedRequest {
            model: String,
            prompt: String,
        }

        #[derive(Deserialize)]
        struct OllamaEmbedResponse {
            embedding: Vec<f32>,
        }

        // Derive base from chat url if possible, fallback
        let base = self.config.api_url.trim_end_matches("/api/chat");
        let url = format!("{}/api/embeddings", base);

        let request = OllamaEmbedRequest {
            model: model.to_string(),
            prompt: text.to_string(),
        };

        let response = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                DbError::ExecutionError(format!("Ollama embeddings request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DbError::ExecutionError(format!(
                "Ollama embeddings error {}: {}",
                status, body
            )));
        }

        let result: OllamaEmbedResponse = response.json().await.map_err(|e| {
            DbError::ExecutionError(format!("Failed to parse Ollama embeddings: {}", e))
        })?;

        Ok(result.embedding)
    }

    async fn embed_gemini(&self, text: &str, model: &str) -> Result<Vec<f32>, DbError> {
        #[derive(Serialize)]
        struct GeminiEmbedRequest {
            content: GeminiEmbedContent,
        }

        #[derive(Serialize)]
        struct GeminiEmbedContent {
            parts: Vec<GeminiEmbedPart>,
        }

        #[derive(Serialize)]
        struct GeminiEmbedPart {
            text: String,
        }

        #[derive(Deserialize)]
        struct GeminiEmbedResponse {
            embedding: Option<GeminiEmbedValues>,
        }

        #[derive(Deserialize)]
        struct GeminiEmbedValues {
            values: Vec<f32>,
        }

        // See the chat path: the key goes in a header so it cannot leak
        // through a reqwest error string.
        let url = format!("{}/{}:embedContent", self.config.api_url, model);

        let request = GeminiEmbedRequest {
            content: GeminiEmbedContent {
                parts: vec![GeminiEmbedPart {
                    text: text.to_string(),
                }],
            },
        };

        let response = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &self.config.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                DbError::ExecutionError(format!("Gemini embeddings request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(DbError::ExecutionError(format!(
                "Gemini embeddings error {}: {}",
                status, body
            )));
        }

        let result: GeminiEmbedResponse = response.json().await.map_err(|e| {
            DbError::ExecutionError(format!("Failed to parse Gemini embeddings: {}", e))
        })?;

        result
            .embedding
            .map(|e| e.values)
            .ok_or_else(|| DbError::ExecutionError("No embedding returned from Gemini".to_string()))
    }
}
