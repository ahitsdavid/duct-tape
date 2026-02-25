use discord_assist_arr_common::ArrClient;
use reqwest::Client;
use serde::Deserialize;
use serenity::builder::{CreateEmbed, CreateMessage};
use serenity::http::Http;
use serenity::model::channel::ChannelType;
use serenity::model::id::{ChannelId, GuildId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info, warn};

const COLOR_GRAB: u32 = 0xf5c518; // yellow
const COLOR_IMPORT: u32 = 0x2ecc71; // green
const COLOR_ALERT_WARN: u32 = 0xe67e22; // orange
const COLOR_ALERT_CRIT: u32 = 0xe74c3c; // red

#[derive(Clone, Copy)]
enum NotificationCategory {
    MediaGrab,
    MediaImport,
    ServerAlert,
}

struct NotificationEvent {
    category: NotificationCategory,
    title: String,
    body: String,
    color: u32,
}

pub struct NotificationStarter {
    pub guild_id: Option<u64>,
    pub poll_interval_secs: u64,
    pub temp_threshold: f64,
    pub sonarr: Option<(String, String)>,
    pub radarr: Option<(String, String)>,
    pub unraid: Option<(String, String)>,
    pub grabs_channel_id: Option<u64>,
    pub imports_channel_id: Option<u64>,
    pub alerts_channel_id: Option<u64>,
    pub fallback_channel_id: Option<u64>,
}

struct ChannelMap {
    grabs: ChannelId,
    imports: ChannelId,
    alerts: ChannelId,
}

struct NotificationManager {
    http: Arc<Http>,
    channels: ChannelMap,
    poll_interval: Duration,
    pollers: Vec<Box<dyn Poller>>,
    shutdown: watch::Receiver<bool>,
}

trait Poller: Send + Sync {
    fn poll(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<NotificationEvent>> + Send + '_>>;
}

async fn resolve_channels(
    http: &Http,
    guild_id: u64,
    grabs_override: Option<u64>,
    imports_override: Option<u64>,
    alerts_override: Option<u64>,
) -> Result<ChannelMap, String> {
    let gid = GuildId::new(guild_id);
    let existing = gid
        .channels(http)
        .await
        .map_err(|e| format!("Failed to fetch guild channels: {e}"))?;

    // Find or create "Notifications" category
    let category_id = if let Some((&id, _)) = existing
        .iter()
        .find(|(_, ch)| ch.kind == ChannelType::Category && ch.name.eq_ignore_ascii_case("notifications"))
    {
        info!("Found existing Notifications category");
        id
    } else {
        let cat = gid
            .create_channel(
                http,
                serenity::builder::CreateChannel::new("Notifications")
                    .kind(ChannelType::Category),
            )
            .await
            .map_err(|e| format!("Failed to create Notifications category: {e}"))?;
        info!("Created Notifications category");
        cat.id
    };

    let grabs = find_or_create(http, gid, &existing, category_id, "media-grabs", grabs_override).await?;
    let imports = find_or_create(http, gid, &existing, category_id, "media-imports", imports_override).await?;
    let alerts = find_or_create(http, gid, &existing, category_id, "server-alerts", alerts_override).await?;

    Ok(ChannelMap {
        grabs,
        imports,
        alerts,
    })
}

async fn find_or_create(
    http: &Http,
    gid: GuildId,
    existing: &HashMap<ChannelId, serenity::model::channel::GuildChannel>,
    category_id: ChannelId,
    name: &str,
    override_id: Option<u64>,
) -> Result<ChannelId, String> {
    if let Some(id) = override_id {
        return Ok(ChannelId::new(id));
    }
    if let Some((&id, _)) = existing
        .iter()
        .find(|(_, ch)| ch.name == name && ch.parent_id == Some(category_id))
    {
        return Ok(id);
    }
    let ch = gid
        .create_channel(
            http,
            serenity::builder::CreateChannel::new(name)
                .kind(ChannelType::Text)
                .category(category_id),
        )
        .await
        .map_err(|e| format!("Failed to create #{name}: {e}"))?;
    info!("Created #{name} channel");
    Ok(ch.id)
}

impl NotificationStarter {
    pub fn start(self, http: Arc<Http>) {
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);

        tokio::spawn(async move {
            let channels = if let Some(fallback) = self.fallback_channel_id {
                // Legacy mode: all events go to one channel
                let ch = ChannelId::new(fallback);
                ChannelMap {
                    grabs: ch,
                    imports: ch,
                    alerts: ch,
                }
            } else if let Some(gid) = self.guild_id {
                match resolve_channels(
                    &http,
                    gid,
                    self.grabs_channel_id,
                    self.imports_channel_id,
                    self.alerts_channel_id,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to set up notification channels: {e}");
                        return;
                    }
                }
            } else {
                error!("Notifications: neither guild_id nor channel_id configured, cannot start");
                return;
            };

            info!(
                "Notification channels: grabs={}, imports={}, alerts={}",
                channels.grabs, channels.imports, channels.alerts
            );
            let mut manager = NotificationManager::new(http, channels, &self, shutdown_rx);
            manager.run().await;
        });
    }
}

impl NotificationManager {
    fn new(
        http: Arc<Http>,
        channels: ChannelMap,
        starter: &NotificationStarter,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        let mut pollers: Vec<Box<dyn Poller>> = Vec::new();

        if let Some((ref url, ref key)) = starter.sonarr {
            pollers.push(Box::new(ArrHistoryPoller::new("Sonarr", url, key, "v3")));
            info!("Notifications: added Sonarr history poller");
        }
        if let Some((ref url, ref key)) = starter.radarr {
            pollers.push(Box::new(ArrHistoryPoller::new("Radarr", url, key, "v3")));
            info!("Notifications: added Radarr history poller");
        }
        if let Some((ref url, ref key)) = starter.unraid {
            pollers.push(Box::new(UnraidPoller::new(url, key, starter.temp_threshold)));
            info!("Notifications: added Unraid poller");
        }

        Self {
            http,
            channels,
            poll_interval: Duration::from_secs(starter.poll_interval_secs),
            pollers,
            shutdown,
        }
    }

    async fn run(&mut self) {
        info!(
            "Notification manager started, polling every {}s",
            self.poll_interval.as_secs()
        );

        loop {
            for poller in &mut self.pollers {
                let events = poller.poll().await;
                for event in events {
                    let channel = match event.category {
                        NotificationCategory::MediaGrab => self.channels.grabs,
                        NotificationCategory::MediaImport => self.channels.imports,
                        NotificationCategory::ServerAlert => self.channels.alerts,
                    };

                    let embed = CreateEmbed::new()
                        .title(&event.title)
                        .description(&event.body)
                        .color(event.color);
                    let message = CreateMessage::new().embed(embed);

                    if let Err(e) = channel.send_message(&self.http, message).await {
                        error!("Failed to send notification to {channel}: {e}");
                    }
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(self.poll_interval) => {}
                _ = self.shutdown.changed() => {
                    info!("Notification manager shutting down");
                    return;
                }
            }
        }
    }
}

// --- Arr History Poller (Sonarr/Radarr) ---

#[derive(Debug, Deserialize)]
struct HistoryResponse {
    records: Vec<HistoryRecord>,
}

#[derive(Debug, Deserialize)]
struct HistoryRecord {
    id: u64,
    #[serde(rename = "eventType")]
    event_type: String,
    #[serde(rename = "sourceTitle")]
    source_title: Option<String>,
}

struct ArrHistoryPoller {
    service_name: String,
    client: ArrClient,
    seen_ids: HashSet<u64>,
    first_poll: bool,
}

impl ArrHistoryPoller {
    fn new(service_name: &str, url: &str, key: &str, api_version: &str) -> Self {
        Self {
            service_name: service_name.to_string(),
            client: ArrClient::with_api_version(url, key, api_version),
            seen_ids: HashSet::new(),
            first_poll: true,
        }
    }
}

impl Poller for ArrHistoryPoller {
    fn poll(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<NotificationEvent>> + Send + '_>>
    {
        Box::pin(async move {
            let result: Result<HistoryResponse, _> = self
                .client
                .get_with_params(
                    "history",
                    &[
                        ("pageSize", "20"),
                        ("sortDirection", "descending"),
                        ("sortKey", "date"),
                    ],
                )
                .await;

            let history = match result {
                Ok(h) => h,
                Err(e) => {
                    warn!("{} history poll failed: {e}", self.service_name);
                    return Vec::new();
                }
            };

            let mut events = Vec::new();

            for record in &history.records {
                if self.first_poll {
                    self.seen_ids.insert(record.id);
                    continue;
                }

                if self.seen_ids.contains(&record.id) {
                    continue;
                }

                self.seen_ids.insert(record.id);

                let title_str = record.source_title.as_deref().unwrap_or("Unknown");

                match record.event_type.as_str() {
                    "grabbed" => {
                        events.push(NotificationEvent {
                            category: NotificationCategory::MediaGrab,
                            title: format!("{} Grab", self.service_name),
                            body: format!("Grabbed: {title_str}"),
                            color: COLOR_GRAB,
                        });
                    }
                    "downloadFolderImported" => {
                        events.push(NotificationEvent {
                            category: NotificationCategory::MediaImport,
                            title: format!("{} Import", self.service_name),
                            body: format!("Imported: {title_str}"),
                            color: COLOR_IMPORT,
                        });
                    }
                    _ => {}
                }
            }

            // Keep seen_ids from growing unbounded
            if self.seen_ids.len() > 1000 {
                let current_ids: HashSet<u64> = history.records.iter().map(|r| r.id).collect();
                self.seen_ids.retain(|id| current_ids.contains(id));
            }

            self.first_poll = false;
            events
        })
    }
}

// --- Unraid Poller ---

#[derive(Debug, Deserialize)]
struct UnraidGraphQLResponse<T> {
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct UnraidPollData {
    array: UnraidArrayState,
    disks: Vec<UnraidDiskInfo>,
    docker: UnraidDockerData,
}

#[derive(Debug, Deserialize)]
struct UnraidArrayState {
    state: String,
}

#[derive(Debug, Deserialize)]
struct UnraidDiskInfo {
    name: String,
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct UnraidDockerData {
    containers: Vec<UnraidContainer>,
}

#[derive(Debug, Deserialize)]
struct UnraidContainer {
    names: Vec<String>,
    state: String,
}

impl UnraidContainer {
    fn display_name(&self) -> &str {
        self.names
            .first()
            .map(|n| n.strip_prefix('/').unwrap_or(n))
            .unwrap_or("unknown")
    }
}

struct UnraidPoller {
    client: Client,
    base_url: String,
    api_key: String,
    temp_threshold: f64,
    last_array_state: Option<String>,
    last_container_states: HashMap<String, String>,
    first_poll: bool,
}

impl UnraidPoller {
    fn new(url: &str, key: &str, temp_threshold: f64) -> Self {
        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client,
            base_url: url.trim_end_matches('/').to_string(),
            api_key: key.to_string(),
            temp_threshold,
            last_array_state: None,
            last_container_states: HashMap::new(),
            first_poll: true,
        }
    }

    async fn query(&self) -> Result<UnraidPollData, String> {
        let query = r#"{
            array { state }
            disks { name temperature }
            docker { containers { names state } }
        }"#;

        let body = serde_json::json!({ "query": query });
        let resp = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let parsed: UnraidGraphQLResponse<UnraidPollData> =
            resp.json().await.map_err(|e| e.to_string())?;

        parsed
            .data
            .ok_or_else(|| "No data in response".to_string())
    }
}

impl Poller for UnraidPoller {
    fn poll(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<NotificationEvent>> + Send + '_>>
    {
        Box::pin(async move {
            let data = match self.query().await {
                Ok(d) => d,
                Err(e) => {
                    warn!("Unraid poll failed: {e}");
                    return Vec::new();
                }
            };

            let mut events = Vec::new();

            if self.first_poll {
                self.last_array_state = Some(data.array.state.clone());
                for container in &data.docker.containers {
                    self.last_container_states
                        .insert(container.display_name().to_string(), container.state.clone());
                }
                self.first_poll = false;
                return events;
            }

            // Check array state transitions
            if let Some(ref last_state) = self.last_array_state
                && *last_state != data.array.state
            {
                events.push(NotificationEvent {
                    category: NotificationCategory::ServerAlert,
                    title: "Array State Changed".into(),
                    body: format!("{} -> {}", last_state, data.array.state),
                    color: COLOR_ALERT_WARN,
                });
            }
            self.last_array_state = Some(data.array.state.clone());

            // Check disk temperatures
            for disk in &data.disks {
                if let Some(temp) = disk.temperature
                    && temp >= self.temp_threshold
                {
                    events.push(NotificationEvent {
                        category: NotificationCategory::ServerAlert,
                        title: "Disk Temperature Warning".into(),
                        body: format!(
                            "{}: {:.0}C (threshold: {:.0}C)",
                            disk.name, temp, self.temp_threshold
                        ),
                        color: COLOR_ALERT_CRIT,
                    });
                }
            }

            // Check container state transitions (detect crashes: RUNNING -> EXITED)
            let mut current_states = HashMap::new();
            for container in &data.docker.containers {
                let name = container.display_name().to_string();
                let state = &container.state;

                if let Some(last_state) = self.last_container_states.get(&name)
                    && last_state == "RUNNING"
                    && state != "RUNNING"
                {
                    events.push(NotificationEvent {
                        category: NotificationCategory::ServerAlert,
                        title: "Container Down".into(),
                        body: format!("{name}: {last_state} -> {state}"),
                        color: COLOR_ALERT_CRIT,
                    });
                }
                current_states.insert(name, state.clone());
            }
            self.last_container_states = current_states;

            events
        })
    }
}
