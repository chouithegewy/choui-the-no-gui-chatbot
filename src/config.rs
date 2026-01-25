use anyhow::{Context, Result};
use std::env;

#[derive(Debug, Clone)]
pub enum LlmProvider {
    Gemini,
    Ollama,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bot_user_id: String,
    pub channel_user_id: Option<String>,
    pub channel_name: Option<String>,

    pub client_id: String,
    // Access token is populated at runtime
    pub oauth_token: Option<String>,

    pub llm_provider: LlmProvider,
    pub gemini_api_key: Option<String>,
    pub gemini_model: String,
    pub ollama_model: String,
    pub ollama_host: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let llm_provider = match env::var("LLM_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "ollama" => LlmProvider::Ollama,
            _ => LlmProvider::Gemini, // Default to Gemini
        };

        Ok(Self {
            bot_user_id: env::var("BOT_USER_ID").context("BOT_USER_ID not set")?,
            channel_user_id: env::var("CHANNEL_USER_ID").ok(),
            channel_name: env::var("CHANNEL_NAME").ok(),
            client_id: env::var("CLIENT_ID").context("CLIENT_ID not set")?,
            oauth_token: None,
            llm_provider,
            gemini_api_key: env::var("GEMINI_API_KEY").ok(),
            gemini_model: env::var("GEMINI_MODEL")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "gemini-2.0-flash".to_string()),
            ollama_model: env::var("OLLAMA_MODEL")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "llama3.2:1b".to_string()),
            ollama_host: env::var("OLLAMA_HOST")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
        })
    }
}
