use async_trait::async_trait;
use discord_assist_plugin_api::{Plugin, PluginError};
use reqwest::Client;
use serde::Deserialize;
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::prelude::Context;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

#[derive(Debug, Deserialize)]
struct TransferInfo {
    dl_info_speed: Option<u64>,
    up_info_speed: Option<u64>,
    dl_info_data: Option<u64>,
    up_info_data: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TorrentInfo {
    name: String,
    hash: String,
    state: String,
    progress: f64,
    size: Option<u64>,
    dlspeed: Option<u64>,
}

struct QbitClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
    logged_in: Arc<RwLock<bool>>,
}

impl QbitClient {
    fn new(base_url: &str, username: &str, password: &str) -> Self {
        let client = Client::builder()
            .cookie_store(true)
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
            logged_in: Arc::new(RwLock::new(false)),
        }
    }

    async fn login(&self) -> Result<(), PluginError> {
        let url = format!("{}/api/v2/auth/login", self.base_url);
        let resp = self
            .client
            .post(&url)
            .form(&[("username", &self.username), ("password", &self.password)])
            .send()
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        let text = resp
            .text()
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        if text.contains("Ok") {
            *self.logged_in.write().await = true;
            debug!("qBittorrent login successful");
            Ok(())
        } else {
            Err(PluginError::ApiError(
                "qBittorrent login failed".to_string(),
            ))
        }
    }

    async fn ensure_logged_in(&self) -> Result<(), PluginError> {
        if !*self.logged_in.read().await {
            self.login().await?;
        }
        Ok(())
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, endpoint: &str) -> Result<T, PluginError> {
        self.ensure_logged_in().await?;
        let url = format!("{}/api/v2{}", self.base_url, endpoint);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::FORBIDDEN {
            // Session expired, re-login and retry
            *self.logged_in.write().await = false;
            self.login().await?;
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| PluginError::ApiError(e.to_string()))?;
            resp.json()
                .await
                .map_err(|e| PluginError::ApiError(e.to_string()))
        } else {
            resp.json()
                .await
                .map_err(|e| PluginError::ApiError(e.to_string()))
        }
    }

    async fn post_form(
        &self,
        endpoint: &str,
        form: &[(&str, &str)],
    ) -> Result<(), PluginError> {
        self.ensure_logged_in().await?;
        let url = format!("{}/api/v2{}", self.base_url, endpoint);
        let resp = self
            .client
            .post(&url)
            .form(form)
            .send()
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::FORBIDDEN {
            *self.logged_in.write().await = false;
            self.login().await?;
            self.client
                .post(&url)
                .form(form)
                .send()
                .await
                .map_err(|e| PluginError::ApiError(e.to_string()))?;
        }
        Ok(())
    }
}

pub struct QbitPlugin {
    client: QbitClient,
}

impl QbitPlugin {
    pub fn new(api_url: &str, username: &str, password: &str) -> Self {
        Self {
            client: QbitClient::new(api_url, username, password),
        }
    }

    async fn handle_status(&self) -> Result<String, PluginError> {
        let info: TransferInfo = self.client.get("/transfer/info").await?;
        let dl = format_speed(info.dl_info_speed.unwrap_or(0));
        let ul = format_speed(info.up_info_speed.unwrap_or(0));
        let dl_total = format_bytes(info.dl_info_data.unwrap_or(0));
        let ul_total = format_bytes(info.up_info_data.unwrap_or(0));
        Ok(format!(
            "**qBittorrent Status**\nDownload: {dl} | Upload: {ul}\nTotal Downloaded: {dl_total} | Total Uploaded: {ul_total}"
        ))
    }

    async fn handle_list(&self) -> Result<String, PluginError> {
        let torrents: Vec<TorrentInfo> = self.client.get("/torrents/info").await?;
        if torrents.is_empty() {
            return Ok("No torrents.".into());
        }
        let mut msg = String::from("**Torrents**\n");
        for t in torrents.iter().take(15) {
            let pct = (t.progress * 100.0) as u32;
            let speed = t.dlspeed.map(|s| format!(" {}", format_speed(s))).unwrap_or_default();
            let size = t.size.map(|s| format!(" ({})", format_bytes(s))).unwrap_or_default();
            msg.push_str(&format!(
                "- **{}**{} — {}% [{}]{}\n",
                truncate(&t.name, 50),
                size,
                pct,
                &t.state,
                speed,
            ));
        }
        if torrents.len() > 15 {
            msg.push_str(&format!("... and {} more\n", torrents.len() - 15));
        }
        Ok(msg)
    }

    async fn handle_pause(&self, name: &str) -> Result<String, PluginError> {
        let hash = self.find_torrent_hash(name).await?;
        self.client
            .post_form("/torrents/pause", &[("hashes", &hash)])
            .await?;
        Ok(format!("Paused torrent matching \"{name}\""))
    }

    async fn handle_resume(&self, name: &str) -> Result<String, PluginError> {
        let hash = self.find_torrent_hash(name).await?;
        self.client
            .post_form("/torrents/resume", &[("hashes", &hash)])
            .await?;
        Ok(format!("Resumed torrent matching \"{name}\""))
    }

    async fn find_torrent_hash(&self, name: &str) -> Result<String, PluginError> {
        let torrents: Vec<TorrentInfo> = self.client.get("/torrents/info").await?;
        let lower = name.to_lowercase();
        let matches: Vec<&TorrentInfo> = torrents
            .iter()
            .filter(|t| t.name.to_lowercase().contains(&lower))
            .collect();

        match matches.len() {
            0 => Err(PluginError::Other(format!(
                "No torrent matching \"{name}\""
            ))),
            1 => Ok(matches[0].hash.clone()),
            n => Err(PluginError::Other(format!(
                "{n} torrents match \"{name}\" — be more specific"
            ))),
        }
    }
}

#[async_trait]
impl Plugin for QbitPlugin {
    fn name(&self) -> &str {
        "qbit"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("qbit")
            .description("qBittorrent torrent management")
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "Show transfer speeds and totals",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "list",
                "List active torrents",
            ))
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "pause",
                    "Pause a torrent by name",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "name",
                        "Torrent name (substring match)",
                    )
                    .required(true),
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "resume",
                    "Resume a paused torrent by name",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "name",
                        "Torrent name (substring match)",
                    )
                    .required(true),
                ),
            )]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "qbit" {
            return Ok(false);
        }

        let options = command.data.options();
        let subopt = match options.first() {
            Some(opt) => opt,
            None => return Ok(false),
        };

        let content = match subopt.name {
            "status" => self.handle_status().await?,
            "list" => self.handle_list().await?,
            "pause" | "resume" => {
                if let ResolvedValue::SubCommand(opts) = &subopt.value {
                    let name = opts
                        .iter()
                        .find(|o| o.name == "name")
                        .and_then(|o| match &o.value {
                            ResolvedValue::String(s) => Some(*s),
                            _ => None,
                        })
                        .ok_or_else(|| PluginError::Other("Missing name".into()))?;

                    if subopt.name == "pause" {
                        self.handle_pause(name).await?
                    } else {
                        self.handle_resume(name).await?
                    }
                } else {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        };

        let data = CreateInteractionResponseMessage::new().content(content);
        let builder = CreateInteractionResponse::Message(data);
        command
            .create_response(&ctx.http, builder)
            .await
            .map_err(PluginError::DiscordError)?;
        Ok(true)
    }
}

fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec < 1024 {
        format!("{} B/s", bytes_per_sec)
    } else if bytes_per_sec < 1_048_576 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{:.1} MB/s", bytes_per_sec as f64 / 1_048_576.0)
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1_073_741_824 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes < 1_099_511_627_776 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else {
        format!("{:.1} TB", bytes as f64 / 1_099_511_627_776.0)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_speed() {
        assert_eq!(format_speed(500), "500 B/s");
        assert_eq!(format_speed(1536), "1.5 KB/s");
        assert_eq!(format_speed(2_621_440), "2.5 MB/s");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(524_288_000), "500.0 MB");
        assert_eq!(format_bytes(1_610_612_736), "1.5 GB");
        assert_eq!(format_bytes(1_649_267_441_664), "1.5 TB");
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let long = "a]".repeat(30);
        let result = truncate(&long, 10);
        assert!(result.len() <= 10);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_multibyte() {
        let s = "hello\u{1F600}world";
        let result = truncate(s, 8);
        assert!(result.ends_with("..."));
        // Must not panic on char boundary
    }
}
