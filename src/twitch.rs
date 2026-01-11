use crate::config::Config;
use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::Path;

pub async fn send_chat_message(message: &str, config: &Config) -> Result<()> {
    // Note: To send chat, we need 'user:write:chat' scope.
    // The device flow requested 'user:read:chat user:write:chat'.

    let token = config.oauth_token.as_ref().context("Token not set")?;
    let broadcaster_id = config
        .channel_user_id
        .as_ref()
        .context("Channel ID not set")?;

    let client = Client::new();
    let body = json!({
        "broadcaster_id": broadcaster_id,
        "sender_id": config.bot_user_id,
        "message": message
    });

    let resp = client
        .post("https://api.twitch.tv/helix/chat/messages")
        .header("Authorization", format!("Bearer {}", token))
        .header("Client-Id", &config.client_id)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await?;
        bail!("Failed to send message ({}): {}", status, text);
    }

    Ok(())
}

pub async fn get_user_id(client: &Client, config: &Config, login: &str) -> Result<String> {
    let token = config.oauth_token.as_ref().context("Token not set")?;

    let resp = client
        .get("https://api.twitch.tv/helix/users")
        .query(&[("login", login)])
        .header("Authorization", format!("Bearer {}", token))
        .header("Client-Id", &config.client_id)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await?;
        bail!("Failed to get user ID: {}", text);
    }

    let json: serde_json::Value = resp.json().await?;
    let id = json["data"][0]["id"]
        .as_str()
        .context("User not found")?
        .to_string();

    Ok(id)
}

pub async fn get_user_login(client: &Client, config: &Config, id: &str) -> Result<String> {
    let token = config.oauth_token.as_ref().context("Token not set")?;

    let resp = client
        .get("https://api.twitch.tv/helix/users")
        .query(&[("id", id)])
        .header("Authorization", format!("Bearer {}", token))
        .header("Client-Id", &config.client_id)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await?;
        bail!("Failed to get user login: {}", text);
    }

    let json: serde_json::Value = resp.json().await?;
    let login = json["data"][0]["login"]
        .as_str()
        .context("User not found")?
        .to_string();

    Ok(login)
}

#[derive(Debug, Deserialize)]
struct DeviceAuthRequest {
    device_code: String,
    user_code: String,
    verification_uri: String,
    _expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    _refresh_token: Option<String>,
    // expires_in: u64,
    // scope: Vec<String>,
}

pub async fn authenticate_via_device_flow(client: &Client, config: &Config) -> Result<String> {
    let scopes = "user:read:chat user:write:chat"; // Required scopes

    // Step 1: Request Device Code
    let params = [("client_id", config.client_id.as_str()), ("scopes", scopes)];

    let resp = client
        .post("https://id.twitch.tv/oauth2/device")
        .form(&params)
        .send()
        .await?;

    if !resp.status().is_success() {
        bail!("Failed to initiate device flow: {}", resp.text().await?);
    }

    let auth_req: DeviceAuthRequest = resp.json().await?;

    println!("*****************************************************************");
    println!("Action Required: Please authenticate the bot.");
    println!("1. Open this URL: {}", auth_req.verification_uri);
    println!("2. Enter this code: {}", auth_req.user_code);
    println!("Waiting for authorization...");
    println!("*****************************************************************");

    // Step 2: Poll for Token
    let interval = std::time::Duration::from_secs(auth_req.interval + 1); // Respect interval
    let mut token_url_params = std::collections::HashMap::new();
    token_url_params.insert("client_id", config.client_id.clone());
    token_url_params.insert("scopes", scopes.to_string());
    token_url_params.insert("device_code", auth_req.device_code.clone());
    token_url_params.insert(
        "grant_type",
        "urn:ietf:params:oauth:grant-type:device_code".to_string(),
    );

    loop {
        tokio::time::sleep(interval).await;

        let token_resp = client
            .post("https://id.twitch.tv/oauth2/token")
            .form(&token_url_params)
            .send()
            .await?;

        if token_resp.status().is_success() {
            let token_data: TokenResponse = token_resp.json().await?;
            // Cache token
            save_token_cache(&token_data.access_token)?;
            return Ok(token_data.access_token);
        } else {
            let error_text = token_resp.text().await?;
            if error_text.contains("authorization_pending") {
                continue;
            } else if error_text.contains("slow_down") {
                // Creating extra delay happens automatically by loop + wait
                continue;
            }
            bail!("Token polling failed: {}", error_text);
        }
    }
}

pub fn save_token_cache(token: &str) -> Result<()> {
    let json = json!({ "access_token": token });
    let mut file = std::fs::File::create(".token_cache.json")?;
    file.write_all(json.to_string().as_bytes())?;
    Ok(())
}

pub fn load_token_cache() -> Result<String> {
    if !Path::new(".token_cache.json").exists() {
        bail!("Cache file not found");
    }
    let data = fs::read_to_string(".token_cache.json")?;
    let json: serde_json::Value = serde_json::from_str(&data)?;
    let token = json["access_token"]
        .as_str()
        .context("No access_token in cache")?;
    Ok(token.to_string())
}

pub async fn subscribe_to_chat_messages(
    client: &Client,
    session_id: &str,
    config: &Config,
) -> Result<()> {
    let token = config.oauth_token.as_ref().context("Token not set")?;

    let body = json!({
        "type": "channel.chat.message",
        "version": "1",
        "condition": {
            "broadcaster_user_id": config.channel_user_id,
            "user_id": config.bot_user_id
        },
        "transport": {
            "method": "websocket",
            "session_id": session_id
        }
    });

    let resp = client
        .post("https://api.twitch.tv/helix/eventsub/subscriptions")
        .header("Authorization", format!("Bearer {}", token))
        .header("Client-Id", &config.client_id)
        .json(&body)
        .send()
        .await?;

    if resp.status() != 202 {
        let status = resp.status();
        let text = resp.text().await?;
        bail!("Subscription failed ({}): {}", status, text);
    }

    let json: serde_json::Value = resp.json().await?;
    let _sub_id = json["data"][0]["id"]
        .as_str()
        .context("No subscription id returned")?;

    Ok(())
}

#[derive(Debug, Deserialize)]
struct EmoteData {
    id: String,
    name: String,
    images: EmoteImages,
}

#[derive(Debug, Deserialize)]
struct EmoteImages {
    url_1x: String,
    url_2x: String,
    url_4x: String,
}

pub async fn get_global_emotes(
    client: &Client,
    config: &Config,
) -> Result<std::collections::HashMap<String, String>> {
    let token = config.oauth_token.as_ref().context("Token not set")?;

    let resp = client
        .get("https://api.twitch.tv/helix/chat/emotes/global")
        .header("Authorization", format!("Bearer {}", token))
        .header("Client-Id", &config.client_id)
        .send()
        .await?;

    if !resp.status().is_success() {
        bail!("Failed to fetch global emotes: {}", resp.status());
    }

    let json: serde_json::Value = resp.json().await?;
    let data = json["data"].as_array().context("Invalid emote format")?;

    let mut map = std::collections::HashMap::new();
    for item in data {
        if let (Some(name), Some(url)) = (item["name"].as_str(), item["images"]["url_1x"].as_str())
        {
            map.insert(name.to_string(), url.to_string());
        }
    }

    Ok(map)
}

pub async fn download_emote(client: &Client, url: &str) -> Result<Vec<u8>> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("Failed to download image: {}", resp.status());
    }
    let bytes = resp.bytes().await?;
    Ok(bytes.to_vec())
}
