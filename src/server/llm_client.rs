//! LLM Client for Natural Language to SDBQL translation
//!
//! Supports OpenAI, Anthropic, and Ollama providers.
//! Reads credentials from _system database's _env collection.

use crate::error::DbError;
use crate::storage::StorageEngine;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Supported LLM providers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LLMProvider {
    OpenAI,
    Anthropic,
    Ollama,
    Gemini,
}

impl LLMProvider {
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
pub struct LLMClient {
    config: LLMConfig,
    http_client: Client,
}

/// Helper to get env var from _system/_env collection or OS environment
fn get_env_var(storage: &StorageEngine, key: &str) -> Option<String> {
    // First, try the database _system/_env collection
    if let Ok(db) = storage.get_database("_system") {
        if let Ok(coll) = db.get_collection("_env") {
            if let Ok(doc) = coll.get(key) {
                if let Some(value) = doc.get("value") {
                    if let Some(s) = value.as_str() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    // Fallback to OS environment variable
    std::env::var(key).ok()
}

impl LLMClient {
    /// Create LLM client from _system/_env collection
    ///
    /// Reads credentials based on provider:
    /// - OpenAI: OPENAI_API_KEY, OPENAI_MODEL (default: gpt-4o)
    /// - Anthropic: ANTHROPIC_API_KEY, ANTHROPIC_MODEL (default: claude-sonnet-4-20250514)
    /// - Ollama: OLLAMA_URL (default: http://localhost:11434), OLLAMA_MODEL (default: llama3)
    /// - Gemini: GEMINI_API_KEY, GEMINI_MODEL (default: gemini-1.5-pro)
    ///
    /// Default provider from NL_DEFAULT_PROVIDER (default: anthropic)
    pub fn from_storage(storage: &StorageEngine, provider: Option<&str>) -> Result<Self, DbError> {
        let provider_str = provider
            .map(|s| s.to_string())
            .or_else(|| get_env_var(storage, "NL_DEFAULT_PROVIDER"))
            .unwrap_or_else(|| "anthropic".to_string());

        let provider = LLMProvider::from_str(&provider_str)?;

        let config = match provider {
            LLMProvider::OpenAI => {
                let api_key = get_env_var(storage, "OPENAI_API_KEY").ok_or_else(|| {
                    DbError::ExecutionError(
                        "OPENAI_API_KEY not found in _system/_env collection".to_string(),
                    )
                })?;
                let model = get_env_var(storage, "OPENAI_MODEL")
                    .unwrap_or_else(|| "gpt-4o".to_string());
                LLMConfig {
                    provider,
                    api_url: "https://api.openai.com/v1/chat/completions".to_string(),
                    api_key,
                    model,
                }
            }
            LLMProvider::Anthropic => {
                let api_key = get_env_var(storage, "ANTHROPIC_API_KEY").ok_or_else(|| {
                    DbError::ExecutionError(
                        "ANTHROPIC_API_KEY not found in _system/_env collection".to_string(),
                    )
                })?;
                let model = get_env_var(storage, "ANTHROPIC_MODEL")
                    .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
                LLMConfig {
                    provider,
                    api_url: "https://api.anthropic.com/v1/messages".to_string(),
                    api_key,
                    model,
                }
            }
            LLMProvider::Ollama => {
                let base_url = get_env_var(storage, "OLLAMA_URL")
                    .unwrap_or_else(|| "http://localhost:11434".to_string());
                let model = get_env_var(storage, "OLLAMA_MODEL")
                    .unwrap_or_else(|| "llama3".to_string());
                LLMConfig {
                    provider,
                    api_url: format!("{}/api/chat", base_url),
                    api_key: String::new(), // Ollama doesn't need API key
                    model,
                }
            }
            LLMProvider::Gemini => {
                let api_key = get_env_var(storage, "GEMINI_API_KEY").ok_or_else(|| {
                    DbError::ExecutionError(
                        "GEMINI_API_KEY not found in _system/_env collection".to_string(),
                    )
                })?;
                let model = get_env_var(storage, "GEMINI_MODEL")
                    .unwrap_or_else(|| "gemini-1.5-pro".to_string());
                LLMConfig {
                    provider,
                    // URL will be constructed dynamically in chat_gemini to include model
                    api_url: "https://generativelanguage.googleapis.com/v1beta/models".to_string(),
                    api_key,
                    model,
                }
            }
        };

        Ok(LLMClient {
            config,
            http_client: Client::new(),
        })
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
            .map_err(|e| DbError::ExecutionError(format!("Ollama API request failed: {}", e)))?;

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
        let system_instruction = messages
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

        let url = format!(
            "{}/{}:generateContent?key={}",
            self.config.api_url, self.config.model, self.config.api_key
        );

        let response = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
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
}
