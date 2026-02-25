use async_trait::async_trait;
use discord_assist_plugin_api::{Plugin, PluginError};
use reqwest::Client;
use serenity::builder::{
    CreateCommand, CreateInteractionResponse, CreateInteractionResponseMessage,
};
use serenity::model::application::CommandInteraction;
use serenity::prelude::Context;
use std::time::Duration;

pub struct ServiceTarget {
    pub name: String,
    pub url: String,
    pub api_key: Option<String>,
    pub key_header: Option<String>,
}

pub struct HealthPlugin {
    services: Vec<ServiceTarget>,
    client: Client,
}

impl HealthPlugin {
    pub fn new(services: Vec<ServiceTarget>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to build HTTP client");
        Self { services, client }
    }

    async fn check_all(&self) -> String {
        let mut lines = vec![String::from("**Service Health**")];

        let mut handles = Vec::new();
        for svc in &self.services {
            let client = self.client.clone();
            let name = svc.name.clone();
            let url = svc.url.clone();
            let api_key = svc.api_key.clone();
            let key_header = svc.key_header.clone();

            handles.push(tokio::spawn(async move {
                let start = std::time::Instant::now();
                let mut req = client.get(&url);
                if let (Some(key), Some(header)) = (&api_key, &key_header) {
                    req = req.header(header.as_str(), key.as_str());
                }
                let result = req.send().await;
                let elapsed = start.elapsed();
                let ms = elapsed.as_millis();

                match result {
                    Ok(resp) if resp.status().is_success() => {
                        format!("- {name}: [UP] ({ms}ms)")
                    }
                    Ok(resp) => {
                        format!("- {name}: [DOWN] (HTTP {})", resp.status().as_u16())
                    }
                    Err(e) if e.is_timeout() => {
                        format!("- {name}: [DOWN] (timeout)")
                    }
                    Err(_) => {
                        format!("- {name}: [DOWN] (connection error)")
                    }
                }
            }));
        }

        for handle in handles {
            if let Ok(line) = handle.await {
                lines.push(line);
            }
        }

        lines.join("\n")
    }
}

#[async_trait]
impl Plugin for HealthPlugin {
    fn name(&self) -> &str {
        "health"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("health").description("Check health of all configured services")]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "health" {
            return Ok(false);
        }

        let content = self.check_all().await;
        let data = CreateInteractionResponseMessage::new().content(content);
        let builder = CreateInteractionResponse::Message(data);
        command
            .create_response(&ctx.http, builder)
            .await
            .map_err(PluginError::DiscordError)?;
        Ok(true)
    }
}
