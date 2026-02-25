pub mod api;

use api::UnraidApi;
use async_trait::async_trait;
use discord_assist_plugin_api::{Plugin, PluginError};
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use serenity::model::application::{
    CommandInteraction, CommandOptionType, ResolvedOption, ResolvedValue,
};
use serenity::prelude::Context;

pub struct UnraidPlugin {
    api: UnraidApi,
}

impl UnraidPlugin {
    pub fn new(api_url: &str, api_key: &str) -> Self {
        Self {
            api: UnraidApi::new(api_url, api_key),
        }
    }
}

#[async_trait]
impl Plugin for UnraidPlugin {
    fn name(&self) -> &str {
        "unraid"
    }

    fn register_commands(&self) -> Vec<CreateCommand> {
        vec![CreateCommand::new("unraid")
            .description("Unraid server management")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommand,
                    "status",
                    "Show array and system status",
                ),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommandGroup,
                    "docker",
                    "Docker container management",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "list",
                        "List all containers",
                    ),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "start",
                        "Start a container",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "name",
                            "Container name",
                        )
                        .required(true),
                    ),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "stop",
                        "Stop a container",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "name",
                            "Container name",
                        )
                        .required(true),
                    ),
                )
                ,
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::SubCommandGroup,
                    "vm",
                    "Virtual machine management",
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "list",
                        "List all VMs",
                    ),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "start",
                        "Start a VM",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "name",
                            "VM name",
                        )
                        .required(true),
                    ),
                )
                .add_sub_option(
                    CreateCommandOption::new(
                        CommandOptionType::SubCommand,
                        "stop",
                        "Stop a VM",
                    )
                    .add_sub_option(
                        CreateCommandOption::new(
                            CommandOptionType::String,
                            "name",
                            "VM name",
                        )
                        .required(true),
                    ),
                ),
            )]
    }

    async fn handle_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<bool, PluginError> {
        if command.data.name != "unraid" {
            return Ok(false);
        }

        let options = command.data.options();
        let subopt = match options.first() {
            Some(opt) => opt,
            None => return Ok(false),
        };

        let content = match &subopt.value {
            ResolvedValue::SubCommand(opts) => {
                self.handle_subcommand("", subopt.name, opts).await?
            }
            ResolvedValue::SubCommandGroup(opts) => {
                if let Some(sub) = opts.first() {
                    if let ResolvedValue::SubCommand(inner) = &sub.value {
                        self.handle_subcommand(subopt.name, sub.name, inner).await?
                    } else {
                        return Ok(false);
                    }
                } else {
                    return Ok(false);
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

impl UnraidPlugin {
    async fn handle_subcommand(
        &self,
        group: &str,
        subcommand: &str,
        options: &[ResolvedOption<'_>],
    ) -> Result<String, PluginError> {
        match (group, subcommand) {
            ("", "status") => {
                let status = self
                    .api
                    .get_system_status()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;

                let uptime = status.info.os.uptime
                    .as_deref()
                    .unwrap_or("unknown");

                let total_storage: f64 = status.disks.iter().map(|d| d.size).sum();
                let total_tb = total_storage / 1_099_511_627_776.0;

                let mut msg = format!(
                    "**{}**\n\
                     Array: {}\n\
                     CPU: {} ({} cores / {} threads)\n\
                     Up since: {}\n\
                     \n**Disks** ({:.1} TB total)\n",
                    status.info.os.hostname,
                    status.array.state,
                    status.info.cpu.brand,
                    status.info.cpu.cores,
                    status.info.cpu.threads,
                    uptime,
                    total_tb,
                );

                for d in &status.disks {
                    let temp = d.temperature
                        .map(|t| format!(" {t:.0}C"))
                        .unwrap_or_default();
                    let size_tb = d.size / 1_099_511_627_776.0;
                    msg.push_str(&format!(
                        "- {} ({:.1} TB) {} [{}]{}\n",
                        d.name, size_tb, d.disk_type, d.smart_status, temp
                    ));
                }

                Ok(msg)
            }
            ("docker", "list") => {
                let containers = self
                    .api
                    .get_docker_containers()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;
                if containers.is_empty() {
                    return Ok("No Docker containers found.".into());
                }
                let mut msg = String::from("**Docker Containers**\n");
                for c in &containers {
                    msg.push_str(&format!("- **{}**: {} ({})\n", c.display_name(), c.state, c.status));
                }
                Ok(msg)
            }
            ("docker", action @ ("start" | "stop")) => {
                let name = options
                    .iter()
                    .find(|o| o.name == "name")
                    .and_then(|o| match &o.value {
                        ResolvedValue::String(s) => Some(*s),
                        _ => None,
                    })
                    .ok_or_else(|| PluginError::Other("Missing container name".into()))?;
                // Look up the container ID by name
                let containers = self
                    .api
                    .get_docker_containers()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;
                let container = containers
                    .iter()
                    .find(|c| c.display_name().eq_ignore_ascii_case(name))
                    .ok_or_else(|| PluginError::Other(format!("Container '{name}' not found")))?;
                let result = self
                    .api
                    .docker_action(&container.id, action)
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;
                Ok(format!("Container **{name}**: {result}"))
            }
            ("vm", "list") => {
                let vms = self
                    .api
                    .get_vms()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;
                if vms.is_empty() {
                    return Ok("No VMs found.".into());
                }
                let mut msg = String::from("**Virtual Machines**\n");
                for vm in &vms {
                    msg.push_str(&format!("- **{}**: {}\n", vm.name, vm.state));
                }
                Ok(msg)
            }
            ("vm", action @ ("start" | "stop")) => {
                let name = options
                    .iter()
                    .find(|o| o.name == "name")
                    .and_then(|o| match &o.value {
                        ResolvedValue::String(s) => Some(*s),
                        _ => None,
                    })
                    .ok_or_else(|| PluginError::Other("Missing VM name".into()))?;
                let result = self
                    .api
                    .vm_action(name, action)
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))?;
                Ok(format!("VM **{name}**: {result}"))
            }
            _ => Ok(format!("Unknown unraid command: {group} {subcommand}")),
        }
    }
}
