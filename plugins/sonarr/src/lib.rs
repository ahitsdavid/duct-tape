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
struct Series {
    title: String,
    year: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct Episode {
    #[serde(rename = "seriesTitle")]
    series_title: Option<String>,
    title: Option<String>,
    #[serde(rename = "airDateUtc")]
    air_date_utc: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QueueStatus {
    #[serde(rename = "totalCount")]
    total_count: Option<u32>,
}

pub struct SonarrPlugin {
    client: ArrClient,
}

impl SonarrPlugin {
    pub fn new(api_url: &str, api_key: &str) -> Self {
        Self {
            client: ArrClient::new(api_url, api_key),
        }
    }
}

#[async_trait]
impl Plugin for SonarrPlugin {
    fn name(&self) -> &str {
        "sonarr"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("sonarr")
            .description("Sonarr TV show management")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "search",
                    "Search for a TV show",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "title",
                        "Show title to search",
                    )
                    .required(true),
                ),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "upcoming",
                "Show upcoming episodes",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "Show queue and system status",
            ))]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "sonarr" {
            return Ok(false);
        }

        let options = command.data.options();
        let subopt = match options.first() {
            Some(opt) => opt,
            None => return Ok(false),
        };

        let content = match subopt.name {
            "search" => {
                if let ResolvedValue::SubCommand(opts) = &subopt.value {
                    let title = opts
                        .iter()
                        .find(|o| o.name == "title")
                        .and_then(|o| match &o.value {
                            ResolvedValue::String(s) => Some(*s),
                            _ => None,
                        })
                        .ok_or_else(|| PluginError::Other("Missing title".into()))?;

                    let results: Vec<Series> = self
                        .client
                        .get_with_params("series/lookup", &[("term", title)])
                        .await
                        .map_err(|e| PluginError::ApiError(e.to_string()))?;

                    if results.is_empty() {
                        format!("No results found for \"{title}\"")
                    } else {
                        let mut msg = format!("**Search results for \"{title}\":**\n");
                        for (i, s) in results.iter().take(10).enumerate() {
                            let year = s.year.map(|y| format!(" ({y})")).unwrap_or_default();
                            msg.push_str(&format!("{}. **{}**{}\n", i + 1, s.title, year));
                        }
                        msg
                    }
                } else {
                    return Ok(false);
                }
            }
            "upcoming" => {
                let episodes: Vec<Episode> = self
                    .client
                    .get("calendar")
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;

                if episodes.is_empty() {
                    "No upcoming episodes.".into()
                } else {
                    let mut msg = String::from("**Upcoming Episodes:**\n");
                    for ep in episodes.iter().take(10) {
                        let series = ep.series_title.as_deref().unwrap_or("Unknown");
                        let title = ep.title.as_deref().unwrap_or("TBA");
                        let date = ep.air_date_utc.as_deref().unwrap_or("TBA");
                        msg.push_str(&format!("- **{series}** — {title} ({date})\n"));
                    }
                    msg
                }
            }
            "status" => {
                let queue: QueueStatus = self
                    .client
                    .get("queue/status")
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;
                let count = queue.total_count.unwrap_or(0);
                format!("**Sonarr Status**\nQueue: {count} items")
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

