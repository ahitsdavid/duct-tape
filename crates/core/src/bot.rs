use discord_assist_plugin_api::Plugin;
use serenity::async_trait;
use serenity::builder::{CreateInteractionResponse, CreateInteractionResponseMessage};
use serenity::model::application::Interaction;
use serenity::model::gateway::Ready;
use serenity::model::id::GuildId;
use serenity::prelude::*;
use tracing::{error, info, warn};

pub struct Bot {
    plugins: Vec<Box<dyn Plugin>>,
    owner_id: u64,
    guild_id: Option<u64>,
}

impl Bot {
    pub fn new(plugins: Vec<Box<dyn Plugin>>, owner_id: u64, guild_id: Option<u64>) -> Self {
        Self {
            plugins,
            owner_id,
            guild_id,
        }
    }

    fn is_owner(&self, user_id: u64) -> bool {
        user_id == self.owner_id
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);

        let mut commands = Vec::new();
        for plugin in &self.plugins {
            let plugin_commands = plugin.register_commands();
            info!(
                "Registering {} commands from plugin '{}'",
                plugin_commands.len(),
                plugin.name()
            );
            commands.extend(plugin_commands);
        }

        if let Some(gid) = self.guild_id {
            let guild_id = GuildId::new(gid);
            match guild_id.set_commands(&ctx.http, commands).await {
                Ok(cmds) => info!("Registered {} guild commands", cmds.len()),
                Err(e) => error!("Failed to register guild commands: {e}"),
            }
        } else {
            for command in commands {
                match serenity::model::application::Command::create_global_command(&ctx.http, command).await {
                    Ok(cmd) => info!("Registered global command: {}", cmd.name),
                    Err(e) => error!("Failed to register global command: {e}"),
                }
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };

        if !self.is_owner(command.user.id.get()) {
            warn!(
                "Unauthorized command attempt by {} ({})",
                command.user.name,
                command.user.id
            );
            let data = CreateInteractionResponseMessage::new()
                .content("You are not authorized to use this bot.")
                .ephemeral(true);
            let builder = CreateInteractionResponse::Message(data);
            let _ = command.create_response(&ctx.http, builder).await;
            return;
        }

        let command_name = command.data.name.clone();
        for plugin in &self.plugins {
            match plugin.handle_command(&ctx, &command).await {
                Ok(true) => return,
                Ok(false) => continue,
                Err(e) => {
                    error!("Plugin '{}' error handling '{}': {e}", plugin.name(), command_name);
                    let data = CreateInteractionResponseMessage::new()
                        .content(e.user_message())
                        .ephemeral(true);
                    let builder = CreateInteractionResponse::Message(data);
                    let _ = command.create_response(&ctx.http, builder).await;
                    return;
                }
            }
        }

        warn!("No plugin handled command: {command_name}");
        let data = CreateInteractionResponseMessage::new()
            .content("Unknown command.")
            .ephemeral(true);
        let builder = CreateInteractionResponse::Message(data);
        let _ = command.create_response(&ctx.http, builder).await;
    }
}
