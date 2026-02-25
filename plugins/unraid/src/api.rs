use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum UnraidApiError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("GraphQL error: {0}")]
    GraphQL(String),
}

#[derive(Clone)]
pub struct UnraidApi {
    client: Client,
    base_url: String,
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

#[derive(Debug, Deserialize)]
pub struct ArrayStatus {
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct SystemStatus {
    pub array: ArrayStatus,
    pub info: SystemInfo,
    pub disks: Vec<DiskInfo>,
}

#[derive(Debug, Deserialize)]
pub struct SystemInfo {
    pub cpu: CpuInfo,
    pub os: OsInfo,
}

#[derive(Debug, Deserialize)]
pub struct CpuInfo {
    pub brand: String,
    pub cores: u32,
    pub threads: u32,
}

#[derive(Debug, Deserialize)]
pub struct OsInfo {
    pub hostname: String,
    pub uptime: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub size: f64,
    pub temperature: Option<f64>,
    #[serde(rename = "smartStatus")]
    pub smart_status: String,
    #[serde(rename = "type")]
    pub disk_type: String,
}

#[derive(Debug, Deserialize)]
pub struct DockerContainer {
    /// Container names (e.g. ["/plex"])
    pub names: Vec<String>,
    pub status: String,
    pub state: String,
    pub id: String,
}

impl DockerContainer {
    /// Get the display name (strips leading slash from first name)
    pub fn display_name(&self) -> &str {
        self.names
            .first()
            .map(|n| n.strip_prefix('/').unwrap_or(n))
            .unwrap_or("unknown")
    }
}

#[derive(Debug, Deserialize)]
pub struct VmDomain {
    pub name: String,
    pub state: String,
}

impl UnraidApi {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("Failed to build HTTP client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        }
    }

    async fn query<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        variables: Option<&serde_json::Value>,
    ) -> Result<T, UnraidApiError> {
        let mut body = serde_json::json!({ "query": query });
        if let Some(vars) = variables {
            body["variables"] = vars.clone();
        }
        let resp = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?
            .json::<GraphQLResponse<T>>()
            .await?;

        if let Some(errors) = resp.errors {
            let msgs: Vec<String> = errors.into_iter().map(|e| e.message).collect();
            return Err(UnraidApiError::GraphQL(msgs.join("; ")));
        }

        resp.data
            .ok_or_else(|| UnraidApiError::GraphQL("No data in response".into()))
    }

    pub async fn get_array_status(&self) -> Result<ArrayStatus, UnraidApiError> {
        #[derive(Deserialize)]
        struct Resp {
            array: ArrayStatus,
        }
        let resp: Resp = self.query("{ array { state } }", None).await?;
        Ok(resp.array)
    }

    pub async fn get_system_status(&self) -> Result<SystemStatus, UnraidApiError> {
        let query = r#"{
            array { state }
            info { cpu { brand cores threads } os { hostname uptime } }
            disks { name size temperature smartStatus type }
        }"#;
        let resp: SystemStatus = self.query(query, None).await?;
        Ok(resp)
    }

    pub async fn get_docker_containers(&self) -> Result<Vec<DockerContainer>, UnraidApiError> {
        #[derive(Deserialize)]
        struct Resp {
            docker: DockerResp,
        }
        #[derive(Deserialize)]
        struct DockerResp {
            containers: Vec<DockerContainer>,
        }
        let resp: Resp = self
            .query(
                "{ docker { containers { id names status state } } }",
                None,
            )
            .await?;
        Ok(resp.docker.containers)
    }

    /// Start/stop a docker container. Uses nested mutation: `mutation { docker { start(id: ...) } }`
    pub async fn docker_action(
        &self,
        id: &str,
        action: &str,
    ) -> Result<String, UnraidApiError> {
        let query = format!(
            "mutation($id: PrefixedID!) {{ docker {{ {action}(id: $id) }} }}"
        );
        let variables = serde_json::json!({ "id": id });
        // The mutation returns a nested structure, but we just need to know it succeeded
        let _: serde_json::Value = self.query(&query, Some(&variables)).await?;
        Ok(format!("{action} succeeded"))
    }

    /// Start/stop a VM. Uses nested mutation: `mutation { vm { start(id: ...) } }`
    pub async fn vm_action(&self, name: &str, action: &str) -> Result<String, UnraidApiError> {
        // VMs use name, not id — we need to look up the domain first
        // For now, pass the name as-is and see if the API accepts it
        let query = format!(
            "mutation($id: PrefixedID!) {{ vm {{ {action}(id: $id) }} }}"
        );
        let variables = serde_json::json!({ "id": name });
        let _: serde_json::Value = self.query(&query, Some(&variables)).await?;
        Ok(format!("{action} succeeded"))
    }

    pub async fn get_vms(&self) -> Result<Vec<VmDomain>, UnraidApiError> {
        #[derive(Deserialize)]
        struct Resp {
            vms: VmsResp,
        }
        #[derive(Deserialize)]
        struct VmsResp {
            domains: Vec<VmDomain>,
        }
        let resp: Resp = self
            .query("{ vms { domains { name state } } }", None)
            .await?;
        Ok(resp.vms.domains)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_get_docker_containers() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "docker": {
                        "containers": [
                            {"id": "abc123", "names": ["/plex"], "status": "Up 2 weeks", "state": "RUNNING"},
                            {"id": "def456", "names": ["/sonarr"], "status": "Up 2 weeks", "state": "RUNNING"}
                        ]
                    }
                }
            })))
            .mount(&mock_server)
            .await;

        let api = UnraidApi::new(&mock_server.uri(), "test-key");
        let containers = api.get_docker_containers().await.unwrap();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0].display_name(), "plex");
        assert_eq!(containers[1].display_name(), "sonarr");
    }

    #[tokio::test]
    async fn test_get_array_status() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "array": { "state": "STARTED" } }
            })))
            .mount(&mock_server)
            .await;

        let api = UnraidApi::new(&mock_server.uri(), "test-key");
        let status = api.get_array_status().await.unwrap();
        assert_eq!(status.state, "STARTED");
    }

    #[tokio::test]
    async fn test_graphql_error_handling() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": null,
                "errors": [{"message": "Unauthorized"}]
            })))
            .mount(&mock_server)
            .await;

        let api = UnraidApi::new(&mock_server.uri(), "bad-key");
        let result = api.get_array_status().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unauthorized"));
    }
}
