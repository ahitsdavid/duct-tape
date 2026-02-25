use async_trait::async_trait;
use discord_assist_plugin_api::{Plugin, PluginError};
use reqwest::Client;
use serde::Deserialize;
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use serenity::model::application::{CommandInteraction, CommandOptionType};
use serenity::prelude::Context;

#[derive(Debug, Deserialize)]
struct MediaContainer<T> {
    #[serde(rename = "MediaContainer")]
    media_container: T,
}

#[derive(Debug, Deserialize)]
struct LibrarySections {
    #[serde(rename = "Directory", default)]
    directories: Vec<LibraryDirectory>,
}

#[derive(Debug, Deserialize)]
struct LibraryDirectory {
    title: String,
    #[serde(rename = "type")]
    lib_type: String,
    key: String,
}

#[derive(Debug, Deserialize)]
struct LibrarySize {
    #[serde(rename = "totalSize", default)]
    total_size: u64,
}

#[derive(Debug, Deserialize)]
struct RecentlyAdded {
    #[serde(rename = "Metadata", default)]
    metadata: Vec<RecentMetadata>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct RecentMetadata {
    title: String,
    #[serde(rename = "parentTitle")]
    parent_title: Option<String>,
    #[serde(rename = "grandparentTitle")]
    grandparent_title: Option<String>,
    #[serde(rename = "addedAt", default)]
    added_at: u64,
    #[serde(rename = "type")]
    media_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Sessions {
    #[serde(rename = "Metadata", default)]
    metadata: Vec<SessionMetadata>,
}

#[derive(Debug, Deserialize)]
struct SessionMetadata {
    title: String,
    #[serde(rename = "grandparentTitle")]
    grandparent_title: Option<String>,
    #[serde(rename = "User")]
    user: Option<SessionUser>,
    #[serde(rename = "Player")]
    player: Option<SessionPlayer>,
}

#[derive(Debug, Deserialize)]
struct SessionUser {
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionPlayer {
    device: Option<String>,
    state: Option<String>,
}

struct PlexClient {
    client: Client,
    base_url: String,
    token: String,
}

impl PlexClient {
    fn new(base_url: &str, token: &str) -> Self {
        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, PluginError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .get(&url)
            .header("X-Plex-Token", &self.token)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(PluginError::ApiError(format!(
                "Plex API returned {}",
                resp.status()
            )));
        }

        resp.json()
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))
    }
}

pub struct PlexPlugin {
    client: PlexClient,
}

impl PlexPlugin {
    pub fn new(api_url: &str, api_key: &str) -> Self {
        Self {
            client: PlexClient::new(api_url, api_key),
        }
    }

    async fn handle_status(&self) -> Result<String, PluginError> {
        let sections: MediaContainer<LibrarySections> =
            self.client.get("/library/sections").await?;

        let mut lines = vec![String::from("**Plex Library Status**")];
        for dir in &sections.media_container.directories {
            let size_resp: MediaContainer<LibrarySize> = self
                .client
                .get(&format!(
                    "/library/sections/{}/all?X-Plex-Container-Size=0",
                    dir.key
                ))
                .await?;
            let count = size_resp.media_container.total_size;
            lines.push(format!("- {}: {} items ({})", dir.title, count, dir.lib_type));
        }
        Ok(lines.join("\n"))
    }

    async fn handle_recent(&self) -> Result<String, PluginError> {
        let recent: MediaContainer<RecentlyAdded> =
            self.client.get("/library/recentlyAdded").await?;

        if recent.media_container.metadata.is_empty() {
            return Ok("No recently added items.".into());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut msg = String::from("**Recently Added**\n");
        for item in recent.media_container.metadata.iter().take(10) {
            let display = match (&item.grandparent_title, &item.parent_title) {
                (Some(show), _) => format!("{show} — {}", item.title),
                _ => item.title.clone(),
            };
            let ago = format_relative_time(now, item.added_at);
            msg.push_str(&format!("- {display} ({ago})\n"));
        }
        Ok(msg)
    }

    async fn handle_streams(&self) -> Result<String, PluginError> {
        let sessions: MediaContainer<Sessions> = self.client.get("/status/sessions").await?;

        if sessions.media_container.metadata.is_empty() {
            return Ok("No active streams.".into());
        }

        let mut msg = String::from("**Active Streams**\n");
        for s in &sessions.media_container.metadata {
            let user = s
                .user
                .as_ref()
                .and_then(|u| u.title.as_deref())
                .unwrap_or("Unknown");
            let device = s
                .player
                .as_ref()
                .and_then(|p| p.device.as_deref())
                .unwrap_or("Unknown");
            let state = s
                .player
                .as_ref()
                .and_then(|p| p.state.as_deref())
                .unwrap_or("unknown");
            let title = match &s.grandparent_title {
                Some(show) => format!("{show} — {}", s.title),
                None => s.title.clone(),
            };
            msg.push_str(&format!("- **{user}**: {title} [{state}] ({device})\n"));
        }
        Ok(msg)
    }
}

#[async_trait]
impl Plugin for PlexPlugin {
    fn name(&self) -> &str {
        "plex"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("plex")
            .description("Plex media server management")
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "Show library counts",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "recent",
                "Show recently added media",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "streams",
                "Show active streams",
            ))]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "plex" {
            return Ok(false);
        }

        let options = command.data.options();
        let subopt = match options.first() {
            Some(opt) => opt,
            None => return Ok(false),
        };

        let content = match subopt.name {
            "status" => self.handle_status().await?,
            "recent" => self.handle_recent().await?,
            "streams" => self.handle_streams().await?,
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

fn format_relative_time(now: u64, timestamp: u64) -> String {
    if timestamp == 0 || timestamp > now {
        return "just now".into();
    }
    let diff = now - timestamp;
    if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_time_minutes() {
        assert_eq!(format_relative_time(1000, 400), "10m ago");
    }

    #[test]
    fn test_relative_time_hours() {
        assert_eq!(format_relative_time(10000, 10000 - 7200), "2h ago");
    }

    #[test]
    fn test_relative_time_days() {
        assert_eq!(format_relative_time(200000, 200000 - 172800), "2d ago");
    }

    #[test]
    fn test_relative_time_future() {
        assert_eq!(format_relative_time(100, 200), "just now");
    }

    #[test]
    fn test_relative_time_zero() {
        assert_eq!(format_relative_time(100, 0), "just now");
    }
}
