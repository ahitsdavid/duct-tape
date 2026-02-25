mod bot;
mod config;

use bot::Bot;
use config::Config;
use discord_assist_plugin_api::Plugin;
use serenity::prelude::*;
use tracing::info;

fn build_plugins(config: &Config) -> Vec<Box<dyn Plugin>> {
    let mut plugins: Vec<Box<dyn Plugin>> = Vec::new();

    if let Some(ref cfg) = config.unraid {
        plugins.push(Box::new(
            discord_assist_unraid::UnraidPlugin::new(&cfg.api_url, &cfg.api_key),
        ));
        info!("Loaded Unraid plugin");
    }

    info!("Loaded {} plugins", plugins.len());
    plugins
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "discord_assist=info".parse().unwrap()),
        )
        .init();

    let config_path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| "config.toml".into());
    let config = Config::load(&config_path)?;

    let plugins = build_plugins(&config);
    let bot = Bot::new(plugins, config.discord.owner_id, config.discord.guild_id);

    let mut client = Client::builder(&config.discord.token, GatewayIntents::empty())
        .event_handler(bot)
        .await?;

    info!("Starting DiscordAssist...");
    client.start().await?;

    Ok(())
}
