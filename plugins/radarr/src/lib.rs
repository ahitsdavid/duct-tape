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
struct Movie {
    title: String,
    year: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct QueueStatus {
    #[serde(rename = "totalCount")]
    total_count: Option<u32>,
}

pub struct RadarrPlugin {
    client: ArrClient,
}

impl RadarrPlugin {
    pub fn new(api_url: &str, api_key: &str) -> Self {
        Self {
            client: ArrClient::new(api_url, api_key),
        }
    }
}

#[async_trait]
impl Plugin for RadarrPlugin {
    fn name(&self) -> &str {
        "radarr"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("radarr")
            .description("Radarr movie management")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "search",
                    "Search for a movie",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::String,
                        "title",
                        "Movie title to search",
                    )
                    .required(true),
                ),
            )
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "upcoming",
                "Show upcoming releases",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::SubCommand,
                "status",
                "Show queue status",
            ))]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "radarr" {
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

                    let encoded = title
                        .replace(' ', "%20")
                        .replace('&', "%26")
                        .replace('=', "%3D");
                    let results: Vec<Movie> = self
                        .client
                        .get(&format!("movie/lookup?term={encoded}"))
                        .await
                        .map_err(|e| PluginError::ApiError(e.to_string()))?;

                    if results.is_empty() {
                        format!("No results found for \"{title}\"")
                    } else {
                        let mut msg = format!("**Search results for \"{title}\":**\n");
                        for (i, m) in results.iter().take(10).enumerate() {
                            let year = m.year.map(|y| format!(" ({y})")).unwrap_or_default();
                            msg.push_str(&format!("{}. **{}**{}\n", i + 1, m.title, year));
                        }
                        msg
                    }
                } else {
                    return Ok(false);
                }
            }
            "upcoming" => {
                let movies: Vec<Movie> = self
                    .client
                    .get("calendar")
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;

                if movies.is_empty() {
                    "No upcoming releases.".into()
                } else {
                    let mut msg = String::from("**Upcoming Releases:**\n");
                    for m in movies.iter().take(10) {
                        let year = m.year.map(|y| format!(" ({y})")).unwrap_or_default();
                        msg.push_str(&format!("- **{}**{}\n", m.title, year));
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
                format!("**Radarr Status**\nQueue: {count} items")
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
        let s = "the matrix";
        let encoded = s
            .replace(' ', "%20")
            .replace('&', "%26")
            .replace('=', "%3D");
        assert_eq!(encoded, "the%20matrix");
    }
}
