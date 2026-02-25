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
pub struct DockerContainer {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VmDomain {
    pub name: String,
    pub state: String,
}

impl UnraidApi {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        }
    }

    async fn query<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
    ) -> Result<T, UnraidApiError> {
        let body = serde_json::json!({ "query": query });
        let resp = self
            .client
            .post(&self.base_url)
            .bearer_auth(&self.api_key)
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
        let resp: Resp = self.query("{ array { state } }").await?;
        Ok(resp.array)
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
            .query("{ docker { containers { name status state } } }")
            .await?;
        Ok(resp.docker.containers)
    }

    pub async fn docker_action(
        &self,
        container: &str,
        action: &str,
    ) -> Result<String, UnraidApiError> {
        let query = format!(
            r#"mutation {{ dockerContainerAction(name: "{container}", action: "{action}") }}"#
        );
        #[derive(Deserialize)]
        struct Resp {
            #[serde(rename = "dockerContainerAction")]
            result: String,
        }
        let resp: Resp = self.query(&query).await?;
        Ok(resp.result)
    }

    pub async fn vm_action(&self, name: &str, action: &str) -> Result<String, UnraidApiError> {
        let query = format!(
            r#"mutation {{ vmAction(name: "{name}", action: "{action}") }}"#
        );
        #[derive(Deserialize)]
        struct Resp {
            #[serde(rename = "vmAction")]
            result: String,
        }
        let resp: Resp = self.query(&query).await?;
        Ok(resp.result)
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
        let resp: Resp = self.query("{ vms { domains { name state } } }").await?;
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
                            {"name": "plex", "status": "running", "state": "running"},
                            {"name": "sonarr", "status": "running", "state": "running"}
                        ]
                    }
                }
            })))
            .mount(&mock_server)
            .await;

        let api = UnraidApi::new(&mock_server.uri(), "test-key");
        let containers = api.get_docker_containers().await.unwrap();
        assert_eq!(containers.len(), 2);
        assert_eq!(containers[0].name, "plex");
    }

    #[tokio::test]
    async fn test_get_array_status() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "array": { "state": "Started" } }
            })))
            .mount(&mock_server)
            .await;

        let api = UnraidApi::new(&mock_server.uri(), "test-key");
        let status = api.get_array_status().await.unwrap();
        assert_eq!(status.state, "Started");
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
