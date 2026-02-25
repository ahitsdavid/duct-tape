mod bot;
mod config;
mod notifications;

use bot::Bot;
use config::Config;
use discord_assist_plugin_api::Plugin;
use notifications::NotificationStarter;
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

    if let Some(ref cfg) = config.claude {
        plugins.push(Box::new(
            discord_assist_claude::ClaudePlugin::new(&cfg.api_url, cfg.api_key.clone()),
        ));
        info!("Loaded Claude plugin");
    }

    if let Some(ref cfg) = config.sonarr {
        plugins.push(Box::new(
            discord_assist_sonarr::SonarrPlugin::new(&cfg.api_url, &cfg.api_key),
        ));
        info!("Loaded Sonarr plugin");
    }

    if let Some(ref cfg) = config.radarr {
        plugins.push(Box::new(
            discord_assist_radarr::RadarrPlugin::new(&cfg.api_url, &cfg.api_key),
        ));
        info!("Loaded Radarr plugin");
    }

    if let Some(ref cfg) = config.prowlarr {
        plugins.push(Box::new(
            discord_assist_prowlarr::ProwlarrPlugin::new(&cfg.api_url, &cfg.api_key),
        ));
        info!("Loaded Prowlarr plugin");
    }

    if let Some(ref cfg) = config.health {
        let services = cfg
            .services
            .iter()
            .map(|s| discord_assist_health::ServiceTarget {
                name: s.name.clone(),
                url: s.url.clone(),
                api_key: s.api_key.clone(),
                key_header: s.key_header.clone(),
            })
            .collect();
        plugins.push(Box::new(discord_assist_health::HealthPlugin::new(services)));
        info!("Loaded Health plugin");
    }

    if let Some(ref cfg) = config.qbit {
        plugins.push(Box::new(discord_assist_qbit::QbitPlugin::new(
            &cfg.api_url,
            &cfg.username,
            &cfg.password,
        )));
        info!("Loaded qBittorrent plugin");
    }

    if let Some(ref cfg) = config.plex {
        plugins.push(Box::new(discord_assist_plex::PlexPlugin::new(
            &cfg.api_url,
            &cfg.api_key,
        )));
        info!("Loaded Plex plugin");
    }

    if let Some(ref req_cfg) = config.request
        && req_cfg.enabled
    {
        if let Some(ref prowlarr) = config.prowlarr {
            let sonarr = config
                .sonarr
                .as_ref()
                .map(|c| (c.api_url.as_str(), c.api_key.as_str()));
            let radarr = config
                .radarr
                .as_ref()
                .map(|c| (c.api_url.as_str(), c.api_key.as_str()));
            plugins.push(Box::new(discord_assist_request::RequestPlugin::new(
                &prowlarr.api_url,
                &prowlarr.api_key,
                sonarr,
                radarr,
            )));
            info!("Loaded Request plugin");
        } else {
            tracing::warn!("Request plugin enabled but [prowlarr] is not configured, skipping");
        }
    }

    if let Some(ref cfg) = config.notes {
        plugins.push(Box::new(discord_assist_notes::NotesPlugin::new(
            &cfg.vault_path,
        )));
        info!("Loaded Notes plugin");
    }

    info!("Loaded {} plugins", plugins.len());
    plugins
}

fn build_notification_starter(config: &Config) -> Option<NotificationStarter> {
    let notif = config.notifications.as_ref()?;

    let sonarr = config
        .sonarr
        .as_ref()
        .map(|c| (c.api_url.clone(), c.api_key.clone()));
    let radarr = config
        .radarr
        .as_ref()
        .map(|c| (c.api_url.clone(), c.api_key.clone()));
    let unraid = config
        .unraid
        .as_ref()
        .map(|c| (c.api_url.clone(), c.api_key.clone()));

    Some(NotificationStarter {
        guild_id: notif.guild_id,
        poll_interval_secs: notif.poll_interval_secs,
        temp_threshold: notif.temp_threshold,
        sonarr,
        radarr,
        unraid,
        grabs_channel_id: notif.grabs_channel_id,
        imports_channel_id: notif.imports_channel_id,
        alerts_channel_id: notif.alerts_channel_id,
        fallback_channel_id: notif.channel_id,
    })
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
    let notification_starter = build_notification_starter(&config);
    let bot = Bot::new(
        plugins,
        config.discord.owner_id,
        config.discord.guild_id,
        notification_starter,
    );

    let mut client = Client::builder(&config.discord.token, GatewayIntents::empty())
        .event_handler(bot)
        .await?;

    info!("Starting DiscordAssist...");
    client.start().await?;

    Ok(())
}
