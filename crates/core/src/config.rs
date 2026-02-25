use serde::Deserialize;
use std::env;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub discord: DiscordConfig,
    #[serde(default)]
    pub unraid: Option<UnraidConfig>,
    #[serde(default)]
    pub claude: Option<ClaudeConfig>,
    #[serde(default)]
    pub sonarr: Option<SonarrConfig>,
    #[serde(default)]
    pub radarr: Option<RadarrConfig>,
    #[serde(default)]
    pub prowlarr: Option<ProwlarrConfig>,
    #[serde(default)]
    pub health: Option<HealthConfig>,
    #[serde(default)]
    pub qbit: Option<QbitConfig>,
    #[serde(default)]
    pub plex: Option<PlexConfig>,
    #[serde(default)]
    pub request: Option<RequestConfig>,
    #[serde(default)]
    pub notifications: Option<NotificationsConfig>,
    #[serde(default)]
    pub notes: Option<NotesConfig>,
}

#[derive(Debug, Deserialize)]
pub struct DiscordConfig {
    pub token: String,
    pub owner_id: u64,
    #[serde(default)]
    pub guild_id: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UnraidConfig {
    pub api_url: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ClaudeConfig {
    pub api_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SonarrConfig {
    pub api_url: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RadarrConfig {
    pub api_url: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProwlarrConfig {
    pub api_url: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HealthConfig {
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServiceConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub key_header: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct QbitConfig {
    pub api_url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PlexConfig {
    pub api_url: String,
    pub api_key: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RequestConfig {
    /// If true (default), the request plugin is enabled.
    /// Requires [prowlarr] to be configured. Optionally uses [sonarr] and [radarr].
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, Clone)]
pub struct NotificationsConfig {
    pub channel_id: u64,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_temp_threshold")]
    pub temp_threshold: f64,
}

fn default_poll_interval() -> u64 {
    60
}

fn default_temp_threshold() -> f64 {
    50.0
}

#[derive(Debug, Deserialize, Clone)]
pub struct NotesConfig {
    pub vault_path: String,
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;
        config.apply_env_overrides();
        Ok(config)
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(val) = env::var("DISCORD_TOKEN") {
            if !val.is_empty() {
                tracing::debug!("Overriding discord.token from DISCORD_TOKEN env var");
                self.discord.token = val;
            }
        }
        if let Some(ref mut unraid) = self.unraid {
            if let Ok(val) = env::var("UNRAID_API_KEY") {
                if !val.is_empty() {
                    tracing::debug!("Overriding unraid.api_key from UNRAID_API_KEY env var");
                    unraid.api_key = val;
                }
            }
        }
        if let Some(ref mut claude) = self.claude {
            if let Ok(val) = env::var("CLAUDE_API_KEY") {
                if !val.is_empty() {
                    tracing::debug!("Overriding claude.api_key from CLAUDE_API_KEY env var");
                    claude.api_key = Some(val);
                }
            }
        }
        if let Some(ref mut sonarr) = self.sonarr {
            if let Ok(val) = env::var("SONARR_API_KEY") {
                if !val.is_empty() {
                    tracing::debug!("Overriding sonarr.api_key from SONARR_API_KEY env var");
                    sonarr.api_key = val;
                }
            }
        }
        if let Some(ref mut radarr) = self.radarr {
            if let Ok(val) = env::var("RADARR_API_KEY") {
                if !val.is_empty() {
                    tracing::debug!("Overriding radarr.api_key from RADARR_API_KEY env var");
                    radarr.api_key = val;
                }
            }
        }
        if let Some(ref mut prowlarr) = self.prowlarr {
            if let Ok(val) = env::var("PROWLARR_API_KEY") {
                if !val.is_empty() {
                    tracing::debug!("Overriding prowlarr.api_key from PROWLARR_API_KEY env var");
                    prowlarr.api_key = val;
                }
            }
        }
        if let Some(ref mut qbit) = self.qbit {
            if let Ok(val) = env::var("QBIT_USERNAME") {
                if !val.is_empty() {
                    tracing::debug!("Overriding qbit.username from QBIT_USERNAME env var");
                    qbit.username = val;
                }
            }
            if let Ok(val) = env::var("QBIT_PASSWORD") {
                if !val.is_empty() {
                    tracing::debug!("Overriding qbit.password from QBIT_PASSWORD env var");
                    qbit.password = val;
                }
            }
        }
        if let Some(ref mut plex) = self.plex {
            if let Ok(val) = env::var("PLEX_API_KEY") {
                if !val.is_empty() {
                    tracing::debug!("Overriding plex.api_key from PLEX_API_KEY env var");
                    plex.api_key = val;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_config() {
        let toml_str = r#"
            [discord]
            token = "test-token"
            owner_id = 123456789
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.discord.token, "test-token");
        assert_eq!(config.discord.owner_id, 123456789);
        assert!(config.unraid.is_none());
        assert!(config.sonarr.is_none());
    }

    #[test]
    fn parse_full_config() {
        let toml_str = r#"
            [discord]
            token = "test-token"
            owner_id = 123456789
            guild_id = 987654321

            [unraid]
            api_url = "https://unraid.local/graphql"
            api_key = "unraid-key"

            [claude]
            api_url = "http://claude:8080"

            [sonarr]
            api_url = "http://sonarr:8989"
            api_key = "sonarr-key"

            [radarr]
            api_url = "http://radarr:7878"
            api_key = "radarr-key"

            [prowlarr]
            api_url = "http://prowlarr:9696"
            api_key = "prowlarr-key"
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.discord.guild_id, Some(987654321));
        assert!(config.unraid.is_some());
        assert!(config.claude.is_some());
        assert!(config.sonarr.is_some());
        assert!(config.radarr.is_some());
        assert!(config.prowlarr.is_some());
    }

    #[test]
    fn parse_new_plugins_config() {
        let toml_str = r#"
            [discord]
            token = "t"
            owner_id = 1

            [health]
            [[health.services]]
            name = "Sonarr"
            url = "http://sonarr:8989"
            api_key = "key"
            key_header = "X-Api-Key"

            [qbit]
            api_url = "http://qbit:8080"
            username = "admin"
            password = "pass"

            [plex]
            api_url = "http://plex:32400"
            api_key = "plex-token"

            [prowlarr]
            api_url = "http://prowlarr:9696"
            api_key = "prowlarr-key"

            [request]

            [notifications]
            channel_id = 1234567890
        "#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let health = config.health.unwrap();
        assert_eq!(health.services.len(), 1);
        assert_eq!(health.services[0].name, "Sonarr");

        let qbit = config.qbit.unwrap();
        assert_eq!(qbit.username, "admin");

        let plex = config.plex.unwrap();
        assert_eq!(plex.api_url, "http://plex:32400");

        let request = config.request.unwrap();
        assert!(request.enabled);

        let notif = config.notifications.unwrap();
        assert_eq!(notif.channel_id, 1234567890);
        assert_eq!(notif.poll_interval_secs, 60);
        assert_eq!(notif.temp_threshold, 50.0);
    }

    #[test]
    fn missing_discord_section_fails() {
        let toml_str = r#"
            [sonarr]
            api_url = "http://sonarr:8989"
            api_key = "key"
        "#;
        let result: Result<Config, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    // Env var tests must run serially since they share process-wide state.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn env_override_discord_token() {
        let _lock = ENV_LOCK.lock().unwrap();
        let toml_str = r#"
            [discord]
            token = "file-token"
            owner_id = 1
        "#;
        let mut config: Config = toml::from_str(toml_str).unwrap();
        env::set_var("DISCORD_TOKEN", "env-token");
        config.apply_env_overrides();
        env::remove_var("DISCORD_TOKEN");
        assert_eq!(config.discord.token, "env-token");
    }

    #[test]
    fn env_override_empty_is_ignored() {
        let _lock = ENV_LOCK.lock().unwrap();
        let toml_str = r#"
            [discord]
            token = "file-token"
            owner_id = 1
        "#;
        let mut config: Config = toml::from_str(toml_str).unwrap();
        env::set_var("DISCORD_TOKEN", "");
        config.apply_env_overrides();
        env::remove_var("DISCORD_TOKEN");
        assert_eq!(config.discord.token, "file-token");
    }

    #[test]
    fn env_override_plugin_key() {
        let _lock = ENV_LOCK.lock().unwrap();
        let toml_str = r#"
            [discord]
            token = "t"
            owner_id = 1

            [sonarr]
            api_url = "http://sonarr:8989"
            api_key = "file-key"
        "#;
        let mut config: Config = toml::from_str(toml_str).unwrap();
        env::set_var("SONARR_API_KEY", "env-key");
        config.apply_env_overrides();
        env::remove_var("SONARR_API_KEY");
        assert_eq!(config.sonarr.unwrap().api_key, "env-key");
    }

    #[test]
    fn env_override_missing_section_is_ignored() {
        let _lock = ENV_LOCK.lock().unwrap();
        let toml_str = r#"
            [discord]
            token = "t"
            owner_id = 1
        "#;
        let mut config: Config = toml::from_str(toml_str).unwrap();
        env::set_var("SONARR_API_KEY", "env-key");
        config.apply_env_overrides();
        env::remove_var("SONARR_API_KEY");
        assert!(config.sonarr.is_none());
    }
}
