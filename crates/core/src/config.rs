use serde::Deserialize;

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

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
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
    fn missing_discord_section_fails() {
        let toml_str = r#"
            [sonarr]
            api_url = "http://sonarr:8989"
            api_key = "key"
        "#;
        let result: Result<Config, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }
}
