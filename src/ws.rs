use crate::config::Config;
use crate::state::AppEvent;
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const EVENTSUB_WS_URL: &str = "wss://eventsub.wss.twitch.tv/ws";

#[derive(Debug, Deserialize)]
struct SessionWelcomePayload {
    session: SessionData,
}
#[derive(Debug, Deserialize)]
struct SessionData {
    id: String,
}
#[derive(Debug, Deserialize)]
struct Envelope {
    metadata: Metadata,
    payload: serde_json::Value,
}
#[derive(Debug, Deserialize)]
struct Metadata {
    message_type: String,
}
#[derive(Debug, Deserialize)]
struct ChatMessageContent {
    text: String,
}
#[derive(Debug, Deserialize)]
struct ChatMessageEvent {
    chatter_user_login: String,
    message: ChatMessageContent,
}

pub async fn connect_eventsub_ws(
    _http_client: Client,
    _config: Config,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) -> Result<(String, tokio::task::JoinHandle<Result<()>>)> {
    let (_ws_stream, _) = tokio_tungstenite::connect_async(EVENTSUB_WS_URL).await?;
    let (mut _write, rx) = _ws_stream.split();

    let session_id = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let session_id_clone = session_id.clone();

    let handle = tokio::spawn(async move {
        // ws_stream is split, rx is 'SplitStream'
        // But to make it cleaner, let's just use raw stream matching logic from previous
        let mut stream = rx.fuse();

        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    let text = text.to_string();
                    // Clean up debug print:
                    // println!("WS: Got text message: {}", text);

                    if let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("debug.log")
                    {
                        use std::io::Write;
                        writeln!(file, "WS Received: {}", text).unwrap_or(());
                    }

                    let envelope: Envelope = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::Error(format!("Parse error: {}", e)));
                            continue;
                        }
                    };

                    match envelope.metadata.message_type.as_str() {
                        "session_welcome" => {
                            if let Ok(welcome) =
                                serde_json::from_value::<SessionWelcomePayload>(envelope.payload)
                            {
                                let mut guard = session_id_clone.lock().await;
                                *guard = welcome.session.id.clone();
                                // println!("Session welcome! ID = {}", *guard);
                            } else {
                                let _ = event_tx
                                    .send(AppEvent::Error("Failed to parse welcome".into()));
                            }
                        }
                        "notification" => {
                            if let Some(event) = envelope.payload.get("event") {
                                match serde_json::from_value::<ChatMessageEvent>(event.clone()) {
                                    Ok(chat) => {
                                        let _ = event_tx.send(AppEvent::ChatMessage {
                                            user: chat.chatter_user_login,
                                            text: chat.message.text,
                                        });
                                    }
                                    Err(e) => {
                                        if let Ok(mut file) = std::fs::OpenOptions::new()
                                            .create(true)
                                            .append(true)
                                            .open("debug.log")
                                        {
                                            use std::io::Write;
                                            writeln!(
                                                file,
                                                "Failed to parse ChatMessageEvent: {} \nJSON: {}",
                                                e, event
                                            )
                                            .unwrap_or(());
                                        }
                                    }
                                }
                            }
                        }
                        "session_keepalive" => {}
                        _ => {}
                    }
                }
                Ok(Message::Ping(_)) => {}
                Ok(Message::Close(_)) => {
                    let _ = event_tx.send(AppEvent::Error("WebSocket closed".into()));
                    break;
                }
                _ => {}
            }
        }
        Ok(())
    });

    // Wait a moment for welcome message
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let session_id = session_id.lock().await.clone();
    if session_id.is_empty() {
        bail!("Did not receive session_welcome message");
    }

    Ok((session_id, handle))
}

// Basic IRC WebSocket Connection
pub async fn connect_irc_ws(
    config: Config,
    event_tx: mpsc::UnboundedSender<AppEvent>,
) -> Result<tokio::task::JoinHandle<()>> {
    let url = "wss://irc-ws.chat.twitch.tv:443";
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .context("Failed to connect to IRC")?;

    let (mut write, mut read) = ws_stream.split();

    // Authenticate
    let token = config.oauth_token.as_ref().context("Token missing")?;
    let channel = config
        .channel_name
        .as_ref()
        .context("Channel name missing")?;

    let caps_cmd =
        Message::Text("CAP REQ :twitch.tv/membership twitch.tv/tags twitch.tv/commands".into());
    write.send(caps_cmd).await?;

    let pass_cmd = Message::Text(format!("PASS oauth:{}", token).into());
    write.send(pass_cmd).await?;

    write.send(Message::Text("NICK infobot".into())).await?;

    let join_cmd = Message::Text(format!("JOIN #{}", channel).into());
    write.send(join_cmd).await?;

    let _ = event_tx.send(AppEvent::Info("IRC Connected - Listening for Joins".into()));

    let handle = tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }

                        if line.starts_with("PING") {
                            let pong = line.replace("PING", "PONG");
                            if let Err(_) = write.send(Message::Text(pong.into())).await {
                                break;
                            }
                            continue;
                        }

                        // Parse JOIN/PART
                        // Format: :<user>!<user>@<user>.tmi.twitch.tv JOIN #<channel>
                        if line.contains(" JOIN #") {
                            if let Some(user) = parse_irc_user(line) {
                                let _ = event_tx.send(AppEvent::UserJoined(user));
                            }
                        } else if line.contains(" PART #") {
                            if let Some(user) = parse_irc_user(line) {
                                let _ = event_tx.send(AppEvent::UserLeft(user));
                            }
                        }
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }
    });

    Ok(handle)
}

fn parse_irc_user(line: &str) -> Option<String> {
    // :username!username@username.tmi.twitch.tv JOIN #channel
    if !line.starts_with(':') {
        return None;
    }
    let end = line.find('!')?;
    if end > 1 {
        Some(line[1..end].to_string())
    } else {
        None
    }
}
