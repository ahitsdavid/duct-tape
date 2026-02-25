pub mod backend;

use async_trait::async_trait;
use backend::{HttpLlmBackend, LlmBackend, Message};
use discord_assist_plugin_api::{Plugin, PluginError};
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateInteractionResponseMessage,
};
use serenity::model::application::{CommandInteraction, CommandOptionType, ResolvedValue};
use serenity::prelude::Context;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const DISCORD_MAX_LEN: usize = 2000;

pub struct ClaudePlugin {
    backend: Box<dyn LlmBackend>,
    conversations: Arc<RwLock<HashMap<u64, Vec<Message>>>>,
}

impl ClaudePlugin {
    pub fn new(api_url: &str, api_key: Option<String>) -> Self {
        Self {
            backend: Box::new(HttpLlmBackend::new(api_url, api_key)),
            conversations: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        let split_at = if remaining.len() <= max_len {
            remaining.len()
        } else {
            remaining[..max_len]
                .rfind('\n')
                .unwrap_or(max_len)
        };
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
        if remaining.starts_with('\n') {
            remaining = &remaining[1..];
        }
    }
    chunks
}

#[async_trait]
impl Plugin for ClaudePlugin {
    fn name(&self) -> &str {
        "claude"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("claude")
            .description("Claude AI assistant")
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "ask", "Ask Claude a question")
                    .add_sub_option(
                        CreateCommandOption::new(CommandOptionType::String, "prompt", "Your question")
                            .required(true),
                    ),
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::SubCommand, "status", "Check Claude backend health"),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommandGroup,
                    "conversation",
                    "Multi-turn conversation management",
                )
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::SubCommand, "start", "Start a new conversation"),
                )
                .add_sub_option(
                    CreateCommandOption::new(CommandOptionType::SubCommand, "end", "End the current conversation"),
                ),
            )]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "claude" {
            return Ok(false);
        }

        let options = command.data.options();
        let subopt = match options.first() {
            Some(opt) => opt,
            None => return Ok(false),
        };

        let channel_id = command.channel_id.get();

        let content = match subopt.name {
            "ask" => {
                if let ResolvedValue::SubCommand(opts) = &subopt.value {
                    let prompt = opts
                        .iter()
                        .find(|o| o.name == "prompt")
                        .and_then(|o| match &o.value {
                            ResolvedValue::String(s) => Some(*s),
                            _ => None,
                        })
                        .ok_or_else(|| PluginError::Other("Missing prompt".into()))?;

                    let mut conversations = self.conversations.write().await;
                    let messages = if let Some(history) = conversations.get_mut(&channel_id) {
                        history.push(Message { role: "user".into(), content: prompt.to_string() });
                        history.clone()
                    } else {
                        vec![Message { role: "user".into(), content: prompt.to_string() }]
                    };
                    drop(conversations);

                    let response = self
                        .backend
                        .complete(&messages)
                        .await
                        .map_err(|e| PluginError::ApiError(e.to_string()))?;

                    let mut conversations = self.conversations.write().await;
                    if let Some(history) = conversations.get_mut(&channel_id) {
                        history.push(Message { role: "assistant".into(), content: response.clone() });
                    }

                    response
                } else {
                    return Ok(false);
                }
            }
            "status" => {
                let healthy = self
                    .backend
                    .health_check()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;
                if healthy {
                    "Claude backend is **online**.".into()
                } else {
                    "Claude backend is **offline**.".into()
                }
            }
            "conversation" => {
                if let ResolvedValue::SubCommandGroup(opts) = &subopt.value {
                    if let Some(sub) = opts.first() {
                        match sub.name {
                            "start" => {
                                let mut conversations = self.conversations.write().await;
                                conversations.insert(channel_id, Vec::new());
                                "Conversation started. Use `/claude ask` to chat. Use `/claude conversation end` to finish.".into()
                            }
                            "end" => {
                                let mut conversations = self.conversations.write().await;
                                if conversations.remove(&channel_id).is_some() {
                                    "Conversation ended.".into()
                                } else {
                                    "No active conversation in this channel.".into()
                                }
                            }
                            _ => "Unknown conversation command.".into(),
                        }
                    } else {
                        return Ok(false);
                    }
                } else {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        };

        let chunks = chunk_message(&content, DISCORD_MAX_LEN);
        let first = chunks.first().cloned().unwrap_or_default();

        let data = CreateInteractionResponseMessage::new().content(&first);
        let builder = CreateInteractionResponse::Message(data);
        command
            .create_response(&ctx.http, builder)
            .await
            .map_err(PluginError::DiscordError)?;

        for chunk in chunks.iter().skip(1) {
            command
                .create_followup(&ctx.http, CreateInteractionResponseFollowup::new().content(chunk))
                .await
                .map_err(PluginError::DiscordError)?;
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_message_short() {
        let chunks = chunk_message("Hello", 2000);
        assert_eq!(chunks, vec!["Hello"]);
    }

    #[test]
    fn test_chunk_message_long() {
        let long = "a".repeat(3000);
        let chunks = chunk_message(&long, 2000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 2000);
        assert_eq!(chunks[1].len(), 1000);
    }

    #[test]
    fn test_chunk_message_splits_at_newline() {
        let text = format!("{}\n{}", "a".repeat(1500), "b".repeat(1000));
        let chunks = chunk_message(&text, 2000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "a".repeat(1500));
        assert_eq!(chunks[1], "b".repeat(1000));
    }
}
