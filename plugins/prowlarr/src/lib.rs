use async_trait::async_trait;
use discord_assist_arr_common::ArrClient;
use discord_assist_plugin_api::{Plugin, PluginError};
use serde::Deserialize;
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::prelude::Context;

#[derive(Debug, Deserialize)]
struct Indexer {
    name: String,
    #[serde(rename = "enableRss")]
    enable_rss: Option<bool>,
    #[serde(rename = "enableSearch")]
    enable_search: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    title: String,
    size: Option<u64>,
    #[serde(rename = "indexer")]
    indexer_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HealthCheck {
    source: Option<String>,
    message: Option<String>,
}

pub struct ProwlarrPlugin {
    client: ArrClient,
}

impl ProwlarrPlugin {
    pub fn new(api_url: &str, api_key: &str) -> Self {
        Self {
            client: ArrClient::with_api_version(api_url, api_key, "v1"),
        }
    }
}

#[async_trait]
impl Plugin for ProwlarrPlugin {
    fn name(&self) -> &str {
        "prowlarr"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("prowlarr")
            .description("Prowlarr indexer management")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "indexers",
                    "List configured indexers",
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "search",
                    "Search across all indexers",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "query",
                        "Search query",
                    )
                    .required(true),
                ),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "Indexer health overview",
            ))]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "prowlarr" {
            return Ok(false);
        }

        let options = command.data.options();
        let subopt = match options.first() {
            Some(opt) => opt,
            None => return Ok(false),
        };

        let content = match subopt.name {
            "indexers" => {
                let indexers: Vec<Indexer> = self
                    .client
                    .get("indexer")
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;

                if indexers.is_empty() {
                    "No indexers configured.".into()
                } else {
                    let mut msg = String::from("**Indexers:**\n");
                    for idx in &indexers {
                        let rss = if idx.enable_rss.unwrap_or(false) {
                            "RSS"
                        } else {
                            ""
                        };
                        let search = if idx.enable_search.unwrap_or(false) {
                            "Search"
                        } else {
                            ""
                        };
                        let features = [rss, search]
                            .iter()
                            .filter(|s| !s.is_empty())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                        let features_str = if features.is_empty() {
                            String::new()
                        } else {
                            format!(" [{}]", features)
                        };
                        msg.push_str(&format!("- **{}**{}\n", idx.name, features_str));
                    }
                    msg
                }
            }
            "search" => {
                if let ResolvedValue::SubCommand(opts) = &subopt.value {
                    let query = opts
                        .iter()
                        .find(|o| o.name == "query")
                        .and_then(|o| match &o.value {
                            ResolvedValue::String(s) => Some(*s),
                            _ => None,
                        })
                        .ok_or_else(|| PluginError::Other("Missing query".into()))?;

                    let encoded = query
                        .replace(' ', "%20")
                        .replace('&', "%26")
                        .replace('=', "%3D");
                    let results: Vec<SearchResult> = self
                        .client
                        .get(&format!("search?query={encoded}"))
                        .await
                        .map_err(|e| PluginError::ApiError(e.to_string()))?;

                    if results.is_empty() {
                        format!("No results for \"{query}\"")
                    } else {
                        let mut msg = format!("**Search results for \"{query}\":**\n");
                        for (i, r) in results.iter().take(10).enumerate() {
                            let size = r
                                .size
                                .map(|s| format!(" ({:.1} MB)", s as f64 / 1_048_576.0))
                                .unwrap_or_default();
                            let indexer = r.indexer_name.as_deref().unwrap_or("unknown");
                            msg.push_str(&format!(
                                "{}. **{}**{} — {}\n",
                                i + 1,
                                r.title,
                                size,
                                indexer
                            ));
                        }
                        msg
                    }
                } else {
                    return Ok(false);
                }
            }
            "status" => {
                let health: Vec<HealthCheck> = self
                    .client
                    .get("health")
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;

                if health.is_empty() {
                    "**Prowlarr Status:** All healthy".into()
                } else {
                    let mut msg = String::from("**Prowlarr Health Issues:**\n");
                    for h in &health {
                        let source = h.source.as_deref().unwrap_or("unknown");
                        let message = h.message.as_deref().unwrap_or("no details");
                        msg.push_str(&format!("- **{source}**: {message}\n"));
                    }
                    msg
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

#[cfg(test)]
mod tests {
    #[test]
    fn test_urlencoding() {
        let s = "ubuntu iso";
        let encoded = s
            .replace(' ', "%20")
            .replace('&', "%26")
            .replace('=', "%3D");
        assert_eq!(encoded, "ubuntu%20iso");
    }
}
