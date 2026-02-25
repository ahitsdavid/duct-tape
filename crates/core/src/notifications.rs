use discord_assist_arr_common::ArrClient;
use reqwest::Client;
use serde::Deserialize;
use serenity::http::Http;
use serenity::model::id::ChannelId;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{error, info, warn};

/// Discord message length limit.
const MAX_MESSAGE_LEN: usize = 2000;

pub struct NotificationStarter {
    pub channel_id: u64,
    pub poll_interval_secs: u64,
    pub temp_threshold: f64,
    pub sonarr: Option<(String, String)>,
    pub radarr: Option<(String, String)>,
    pub unraid: Option<(String, String)>,
}

impl NotificationStarter {
    pub fn start(self, http: Arc<Http>) {
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);

        tokio::spawn(async move {
            let mut manager = NotificationManager::from_starter(self, http, shutdown_rx);
            manager.run().await;
        });
    }
}

struct NotificationManager {
    http: Arc<Http>,
    channel_id: ChannelId,
    poll_interval: Duration,
    pollers: Vec<Box<dyn Poller>>,
    shutdown: watch::Receiver<bool>,
}

trait Poller: Send + Sync {
    fn poll(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<NotificationEvent>> + Send + '_>>;
}

struct NotificationEvent {
    title: String,
    body: String,
}

impl NotificationManager {
    fn from_starter(
        starter: NotificationStarter,
        http: Arc<Http>,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        let mut pollers: Vec<Box<dyn Poller>> = Vec::new();

        if let Some((url, key)) = starter.sonarr {
            pollers.push(Box::new(ArrHistoryPoller::new("Sonarr", &url, &key, "v3")));
            info!("Notifications: added Sonarr history poller");
        }
        if let Some((url, key)) = starter.radarr {
            pollers.push(Box::new(ArrHistoryPoller::new("Radarr", &url, &key, "v3")));
            info!("Notifications: added Radarr history poller");
        }
        if let Some((url, key)) = starter.unraid {
            pollers.push(Box::new(UnraidPoller::new(
                &url,
                &key,
                starter.temp_threshold,
            )));
            info!("Notifications: added Unraid poller");
        }

        Self {
            http,
            channel_id: ChannelId::new(starter.channel_id),
            poll_interval: Duration::from_secs(starter.poll_interval_secs),
            pollers,
            shutdown,
        }
    }

    async fn run(&mut self) {
        info!(
            "Notification manager started, polling every {}s to channel {}",
            self.poll_interval.as_secs(),
            self.channel_id
        );

        loop {
            for poller in &mut self.pollers {
                let events = poller.poll().await;
                for event in events {
                    let mut msg = format!("**[{}]** {}", event.title, event.body);
                    msg.truncate(MAX_MESSAGE_LEN);
                    if let Err(e) = self
                        .channel_id
                        .say(&self.http, &msg)
                        .await
                    {
                        error!("Failed to send notification: {e}");
                    }
                }
            }

            // Wait for poll interval or shutdown signal
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
                    &[("pageSize", "20"), ("sortDirection", "descending"), ("sortKey", "date")],
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

                let title_str = record
                    .source_title
                    .as_deref()
                    .unwrap_or("Unknown");

                match record.event_type.as_str() {
                    "grabbed" => {
                        events.push(NotificationEvent {
                            title: format!("{} Grab", self.service_name),
                            body: format!("Grabbed: {title_str}"),
                        });
                    }
                    "downloadFolderImported" => {
                        events.push(NotificationEvent {
                            title: format!("{} Import", self.service_name),
                            body: format!("Imported: {title_str}"),
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

        parsed.data.ok_or_else(|| "No data in response".to_string())
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
            if let Some(ref last_state) = self.last_array_state {
                if *last_state != data.array.state {
                    events.push(NotificationEvent {
                        title: "Unraid Array".into(),
                        body: format!("State changed: {} -> {}", last_state, data.array.state),
                    });
                }
            }
            self.last_array_state = Some(data.array.state.clone());

            // Check disk temperatures
            for disk in &data.disks {
                if let Some(temp) = disk.temperature {
                    if temp >= self.temp_threshold {
                        events.push(NotificationEvent {
                            title: "Unraid Disk Temp".into(),
                            body: format!(
                                "{}: {:.0}C (threshold: {:.0}C)",
                                disk.name, temp, self.temp_threshold
                            ),
                        });
                    }
                }
            }

            // Check container state transitions (detect crashes: RUNNING -> EXITED)
            let mut current_states = HashMap::new();
            for container in &data.docker.containers {
                let name = container.display_name().to_string();
                let state = &container.state;

                if let Some(last_state) = self.last_container_states.get(&name) {
                    if last_state == "RUNNING" && state != "RUNNING" {
                        events.push(NotificationEvent {
                            title: "Unraid Container".into(),
                            body: format!("{name}: {last_state} -> {state}"),
                        });
                    }
                }
                current_states.insert(name, state.clone());
            }
            self.last_container_states = current_states;

            events
        })
    }
}
