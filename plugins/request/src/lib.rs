use async_trait::async_trait;
use discord_assist_arr_common::ArrClient;
use discord_assist_plugin_api::{Plugin, PluginError};
use serde::Deserialize;
use serenity::builder::{
    CreateActionRow, CreateButton, CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateSelectMenu, CreateSelectMenuKind,
    CreateSelectMenuOption,
};
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ComponentInteraction,
    ComponentInteractionDataKind, ResolvedValue,
};
use serenity::prelude::Context;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Deserialize)]
struct ProwlarrResult {
    title: String,
    size: Option<u64>,
    #[serde(rename = "indexer")]
    indexer_name: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingRequest {
    results: Vec<PendingItem>,
    created_at: std::time::Instant,
}

#[derive(Debug, Clone)]
struct PendingItem {
    title: String,
    size: Option<u64>,
    indexer: String,
}

#[derive(Debug, Deserialize)]
struct RootFolder {
    path: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct QualityProfile {
    id: u32,
    name: String,
}

pub struct RequestPlugin {
    prowlarr: ArrClient,
    sonarr: Option<ArrClient>,
    radarr: Option<ArrClient>,
    pending: Arc<RwLock<HashMap<String, PendingRequest>>>,
}

impl RequestPlugin {
    pub fn new(
        prowlarr_url: &str,
        prowlarr_key: &str,
        sonarr: Option<(&str, &str)>,
        radarr: Option<(&str, &str)>,
    ) -> Self {
        Self {
            prowlarr: ArrClient::with_api_version(prowlarr_url, prowlarr_key, "v1"),
            sonarr: sonarr.map(|(url, key)| ArrClient::new(url, key)),
            radarr: radarr.map(|(url, key)| ArrClient::new(url, key)),
            pending: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn cleanup_expired(&self) {
        let mut pending = self.pending.write().await;
        pending.retain(|_, req| req.created_at.elapsed().as_secs() < 900);
    }

    async fn handle_search(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
        title: &str,
    ) -> Result<(), PluginError> {
        self.cleanup_expired().await;

        let results: Vec<ProwlarrResult> = self
            .prowlarr
            .get_with_params("search", &[("query", title)])
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        if results.is_empty() {
            let data = CreateInteractionResponseMessage::new()
                .content(format!("No results found for \"{title}\""));
            command
                .create_response(&ctx.http, CreateInteractionResponse::Message(data))
                .await
                .map_err(PluginError::DiscordError)?;
            return Ok(());
        }

        let id = format!("{}", command.id);
        let items: Vec<PendingItem> = results
            .iter()
            .take(25)
            .map(|r| PendingItem {
                title: r.title.clone(),
                size: r.size,
                indexer: r.indexer_name.clone().unwrap_or_else(|| "unknown".into()),
            })
            .collect();

        let options: Vec<CreateSelectMenuOption> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let size_str = item
                    .size
                    .map(|s| format!(" ({:.1} MB)", s as f64 / 1_048_576.0))
                    .unwrap_or_default();
                let label = truncate_string(&item.title, 100);
                let desc = format!("{}{}", item.indexer, size_str);
                CreateSelectMenuOption::new(label, format!("{i}")).description(truncate_string(&desc, 100))
            })
            .collect();

        self.pending.write().await.insert(
            id.clone(),
            PendingRequest {
                results: items,
                created_at: std::time::Instant::now(),
            },
        );

        let select = CreateSelectMenu::new(
            format!("req_sel:{id}"),
            CreateSelectMenuKind::String { options },
        )
        .placeholder("Select a result...");

        let data = CreateInteractionResponseMessage::new()
            .content(format!("**Search results for \"{title}\":**"))
            .components(vec![CreateActionRow::SelectMenu(select)]);

        command
            .create_response(&ctx.http, CreateInteractionResponse::Message(data))
            .await
            .map_err(PluginError::DiscordError)?;
        Ok(())
    }

    async fn handle_select(
        &self,
        ctx: &Context,
        component: &ComponentInteraction,
        id: &str,
        index: usize,
    ) -> Result<(), PluginError> {
        let pending = self.pending.read().await;
        let req = pending.get(id).ok_or_else(|| {
            PluginError::Other("This request has expired. Please search again.".into())
        })?;

        let item = req.results.get(index).ok_or_else(|| {
            PluginError::Other("Invalid selection.".into())
        })?;

        let mut buttons = Vec::new();
        if self.sonarr.is_some() {
            buttons.push(CreateButton::new(format!("req_add:{id}:sonarr:{index}")).label("Add to Sonarr"));
        }
        if self.radarr.is_some() {
            buttons.push(CreateButton::new(format!("req_add:{id}:radarr:{index}")).label("Add to Radarr"));
        }

        if buttons.is_empty() {
            let data = CreateInteractionResponseMessage::new()
                .content("No target services configured (Sonarr/Radarr).")
                .ephemeral(true);
            component
                .create_response(&ctx.http, CreateInteractionResponse::Message(data))
                .await
                .map_err(PluginError::DiscordError)?;
            return Ok(());
        }

        let size_str = item
            .size
            .map(|s| format!(" ({:.1} MB)", s as f64 / 1_048_576.0))
            .unwrap_or_default();

        let data = CreateInteractionResponseMessage::new()
            .content(format!(
                "**Selected:** {}{}\nWhere would you like to add it?",
                item.title, size_str
            ))
            .components(vec![CreateActionRow::Buttons(buttons)]);

        component
            .create_response(&ctx.http, CreateInteractionResponse::Message(data))
            .await
            .map_err(PluginError::DiscordError)?;
        Ok(())
    }

    async fn handle_add(
        &self,
        ctx: &Context,
        component: &ComponentInteraction,
        id: &str,
        service: &str,
        index: usize,
    ) -> Result<(), PluginError> {
        let pending = self.pending.read().await;
        let req = pending.get(id).ok_or_else(|| {
            PluginError::Other("This request has expired. Please search again.".into())
        })?;

        let item = req.results.get(index).ok_or_else(|| {
            PluginError::Other("Invalid selection.".into())
        })?;

        let client = match service {
            "sonarr" => self.sonarr.as_ref(),
            "radarr" => self.radarr.as_ref(),
            _ => None,
        }
        .ok_or_else(|| PluginError::Other(format!("{service} is not configured")))?;

        // Get root folder and quality profile defaults
        let root_folders: Vec<RootFolder> = client
            .get("rootfolder")
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        let root_path = root_folders
            .first()
            .map(|r| r.path.clone())
            .ok_or_else(|| PluginError::Other(format!("No root folder configured in {service}")))?;

        let profiles: Vec<QualityProfile> = client
            .get("qualityprofile")
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        let profile_id = profiles
            .first()
            .map(|p| p.id)
            .ok_or_else(|| PluginError::Other(format!("No quality profile configured in {service}")))?;

        // Search the target service for this title to get proper metadata
        let search_endpoint = match service {
            "sonarr" => "series/lookup",
            "radarr" => "movie/lookup",
            _ => unreachable!(),
        };

        let search_results: Vec<serde_json::Value> = client
            .get_with_params(search_endpoint, &[("term", item.title.as_str())])
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        let result = search_results
            .first()
            .ok_or_else(|| PluginError::Other(format!("Could not find \"{}\" in {service}", item.title)))?;

        // Build the add request
        let mut add_body = result.clone();
        if let Some(obj) = add_body.as_object_mut() {
            obj.insert("rootFolderPath".into(), serde_json::json!(root_path));
            obj.insert("qualityProfileId".into(), serde_json::json!(profile_id));
            obj.insert("monitored".into(), serde_json::json!(true));
            if service == "sonarr" {
                obj.insert("addOptions".into(), serde_json::json!({"searchForMissingEpisodes": true}));
            } else {
                obj.insert("addOptions".into(), serde_json::json!({"searchForMovie": true}));
            }
        }

        let add_endpoint = match service {
            "sonarr" => "series",
            "radarr" => "movie",
            _ => unreachable!(),
        };

        let _: serde_json::Value = client
            .post(add_endpoint, &add_body)
            .await
            .map_err(|e| PluginError::ApiError(e.to_string()))?;

        let service_name = match service {
            "sonarr" => "Sonarr",
            "radarr" => "Radarr",
            _ => service,
        };

        let data = CreateInteractionResponseMessage::new()
            .content(format!("Added **{}** to {service_name}!", item.title));
        component
            .create_response(&ctx.http, CreateInteractionResponse::Message(data))
            .await
            .map_err(PluginError::DiscordError)?;

        // Cleanup this pending request
        drop(pending);
        self.pending.write().await.remove(id);
        Ok(())
    }
}

#[async_trait]
impl Plugin for RequestPlugin {
    fn name(&self) -> &str {
        "request"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("request")
            .description("Search and add media to Sonarr/Radarr")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "title",
                    "Title to search for",
                )
                .required(true),
            )]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "request" {
            return Ok(false);
        }

        let title = command
            .data
            .options()
            .iter()
            .find(|o| o.name == "title")
            .and_then(|o| match &o.value {
                ResolvedValue::String(s) => Some(*s),
                _ => None,
            })
            .ok_or_else(|| PluginError::Other("Missing title".into()))?;

        self.handle_search(ctx, command, title).await?;
        Ok(true)
    }

    async fn handle_component(
        &self,
        ctx: &Context,
        component: &ComponentInteraction,
    ) -> Result<bool, PluginError> {
        let custom_id = &component.data.custom_id;

        if let Some(rest) = custom_id.strip_prefix("req_sel:") {
            // Select menu: req_sel:<id>
            let id = rest;
            let values = match &component.data.kind {
                ComponentInteractionDataKind::StringSelect { values } => values,
                _ => return Ok(false),
            };
            let index: usize = values
                .first()
                .and_then(|v: &String| v.parse().ok())
                .ok_or_else(|| PluginError::Other("Invalid selection".into()))?;
            self.handle_select(ctx, component, id, index).await?;
            Ok(true)
        } else if let Some(rest) = custom_id.strip_prefix("req_add:") {
            // Button: req_add:<id>:<service>:<index>
            let parts: Vec<&str> = rest.splitn(3, ':').collect();
            if parts.len() != 3 {
                return Ok(false);
            }
            let id = parts[0];
            let service = parts[1];
            let index: usize = parts[2]
                .parse()
                .map_err(|_| PluginError::Other("Invalid index".into()))?;
            self.handle_add(ctx, component, id, service, index).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn truncate_string(s: &str, max: usize) -> String {
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
    fn test_truncate_string_short() {
        assert_eq!(truncate_string("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_string_exact() {
        assert_eq!(truncate_string("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_string_long() {
        let result = truncate_string("hello world!", 8);
        assert_eq!(result, "hello...");
        assert!(result.len() <= 8);
    }

    #[test]
    fn test_truncate_string_multibyte() {
        let s = "hello \u{1F600} world"; // emoji is 4 bytes
        let result = truncate_string(s, 10);
        assert!(result.ends_with("..."));
        // Must not panic on char boundary
    }
}
