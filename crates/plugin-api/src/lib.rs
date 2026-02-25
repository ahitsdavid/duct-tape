use async_trait::async_trait;
use serenity::builder::CreateCommand;
use serenity::model::application::{CommandInteraction, ComponentInteraction};
use serenity::prelude::Context;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("API request failed: {0}")]
    ApiError(String),
    #[error("configuration error: {0}")]
    ConfigError(String),
    #[error("Discord response failed: {0}")]
    DiscordError(#[from] serenity::Error),
    #[error("{0}")]
    Other(String),
}

impl PluginError {
    /// Returns a safe, category-based message suitable for sending to Discord.
    /// Full error details remain available via `Display` (used in server-side logs).
    pub fn user_message(&self) -> &str {
        match self {
            Self::ApiError(_) => "A plugin API request failed. Check bot logs for details.",
            Self::ConfigError(_) => "Plugin configuration error. Check bot logs for details.",
            Self::DiscordError(_) => "Discord API error. Check bot logs for details.",
            Self::Other(_) => "Something went wrong. Check bot logs for details.",
        }
    }
}

/// Trait that all plugins must implement.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Unique name for this plugin (used in logging).
    fn name(&self) -> &str;

    /// Return slash command definitions to register with Discord.
    fn register_commands(&self) -> Vec<CreateCommand>;

    /// Handle an incoming command interaction.
    /// Return Ok(true) if this plugin handled the command, Ok(false) if not.
    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError>;

    /// Handle a component interaction (buttons, select menus).
    /// Return Ok(true) if this plugin handled the interaction, Ok(false) if not.
    async fn handle_component(
        &self,
        _ctx: &Context,
        _component: &ComponentInteraction,
    ) -> Result<bool, PluginError> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_error_display() {
        let err = PluginError::ApiError("timeout".into());
        assert_eq!(err.to_string(), "API request failed: timeout");

        let err = PluginError::ConfigError("missing key".into());
        assert_eq!(err.to_string(), "configuration error: missing key");
    }
}
