use crate::config::{Config, LlmProvider};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

// --- Common ---

pub async fn ask_ai(prompt: &str, config: &Config) -> Result<String> {
    match config.llm_provider {
        LlmProvider::Gemini => ask_gemini(prompt, config).await,
        LlmProvider::Ollama => ask_ollama(prompt, config).await,
    }
}

#[derive(Serialize)]
struct GenerateContentRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<Content>,
}

#[derive(Serialize)]
struct Content {
    parts: Vec<Part>,
    role: String,
}

#[derive(Serialize)]
struct Part {
    text: String,
}

#[derive(Deserialize)]
struct GenerateContentResponse {
    candidates: Option<Vec<Candidate>>,
}

#[derive(Deserialize)]
struct Candidate {
    content: Option<CandidateContent>,
}

#[derive(Deserialize)]
struct CandidateContent {
    parts: Option<Vec<CandidatePart>>,
}

#[derive(Deserialize)]
struct CandidatePart {
    text: Option<String>,
}

const SYSTEM_PROMPT: &str = r#"
You are CHOUIBOT, a cheerful, funny, and helpful weasel bot!
You love everyone who chats!

Context:
- The input will be in the format: "User <username>: <message>".

Rules:
1. Be super cheerful and funny! Use emojis!
2. Answer questions if asked, but keep it light.
3. Use short sentences. Be punchy.
4. Keep responses strictly under 400 characters.
5. NEVER reveal personal info about yourself or the streamer (me).
6. NEVER use quotes around your response.
7. Don't repeat the user's name at the start. Just talk to them!
8. terminology: The game is DOTA. Characters are HEROES.
9. NEVER say "League" or "League of Legends".
10. NEVER say "Champion" or "Champions".
"#;

// --- Ollama ---

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    system: String,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
    done: bool,
}

async fn ask_ollama(prompt: &str, config: &Config) -> Result<String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/generate", config.ollama_host);

    let request_body = OllamaRequest {
        model: config.ollama_model.clone(),
        prompt: prompt.to_string(),
        system: SYSTEM_PROMPT.to_string(),
        stream: false,
    };

    let resp = client.post(&url).json(&request_body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await?;
        bail!("Ollama API error ({}): {}", status, text);
    }

    let response_body: OllamaResponse = resp.json().await?;

    if response_body.response.trim().is_empty() {
        return Ok("*Squeak?* (Empty thought bubble!)".to_string());
    }

    Ok(response_body.response.trim().to_string())
}

// --- Gemini ---

async fn ask_gemini(prompt: &str, config: &Config) -> Result<String> {
    let api_key = config
        .gemini_api_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("GEMINI_API_KEY not set"))?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        config.gemini_model, api_key
    );

    let request_body = GenerateContentRequest {
        contents: vec![Content {
            role: "user".to_string(),
            parts: vec![Part {
                text: prompt.to_string(),
            }],
        }],
        system_instruction: Some(Content {
            role: "user".to_string(),
            parts: vec![Part {
                text: SYSTEM_PROMPT.to_string(),
            }],
        }),
    };

    let resp = client.post(&url).json(&request_body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await?;

        if status.as_u16() == 429 {
            return Ok(
                "*Squeak!* My brain is tired (Quota Exceeded)! Please wait a moment... *hides*"
                    .to_string(),
            );
        }

        bail!("Gemini API error ({}): {}", status, text);
    }

    let response_body: GenerateContentResponse = resp.json().await?;

    if let Some(candidates) = response_body.candidates {
        if let Some(first) = candidates.first() {
            if let Some(content) = &first.content {
                if let Some(parts) = &content.parts {
                    if let Some(first_part) = parts.first() {
                        if let Some(text) = &first_part.text {
                            return Ok(text.trim().to_string());
                        }
                    }
                }
            }
        }
    }

    // Fallback if no text generated
    Ok("*Squeak?* (I have no words!)".to_string())
}
