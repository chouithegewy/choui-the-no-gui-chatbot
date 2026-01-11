use anyhow::{Context, Result};
use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub bot_user_id: String,
    pub channel_user_id: Option<String>,
    pub channel_name: Option<String>,

    pub client_id: String,
    // Access token is populated at runtime
    pub oauth_token: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            bot_user_id: env::var("BOT_USER_ID").context("BOT_USER_ID not set")?,
            channel_user_id: env::var("CHANNEL_USER_ID").ok(),
            channel_name: env::var("CHANNEL_NAME").ok(),

            client_id: env::var("CLIENT_ID").context("CLIENT_ID not set")?,
            oauth_token: None,
        })
    }
}
