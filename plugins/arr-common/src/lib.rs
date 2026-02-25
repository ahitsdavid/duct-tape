use reqwest::Client;
use serde::de::DeserializeOwned;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ArrError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error ({status}): {body}")]
    Api { status: u16, body: String },
}

#[derive(Clone)]
pub struct ArrClient {
    client: Client,
    base_url: String,
    api_key: String,
    api_version: String,
}

impl ArrClient {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self::with_api_version(base_url, api_key, "v3")
    }

    pub fn with_api_version(base_url: &str, api_key: &str, api_version: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            api_version: api_version.to_string(),
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T, ArrError> {
        let url = format!("{}/api/{}/{}", self.base_url, self.api_version, endpoint.trim_start_matches('/'));
        let resp = self
            .client
            .get(&url)
            .header("X-Api-Key", &self.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ArrError::Api { status, body });
        }

        Ok(resp.json().await?)
    }

    pub async fn post<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        body: &serde_json::Value,
    ) -> Result<T, ArrError> {
        let url = format!("{}/api/{}/{}", self.base_url, self.api_version, endpoint.trim_start_matches('/'));
        let resp = self
            .client
            .post(&url)
            .header("X-Api-Key", &self.api_key)
            .json(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ArrError::Api { status, body });
        }

        Ok(resp.json().await?)
    }

    pub async fn health(&self) -> Result<bool, ArrError> {
        let url = format!("{}/api/{}/health", self.base_url, self.api_version);
        let resp = self
            .client
            .get(&url)
            .header("X-Api-Key", &self.api_key)
            .send()
            .await;
        match resp {
            Ok(r) => Ok(r.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_get_with_api_key() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/system/status"))
            .and(header("X-Api-Key", "test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"version": "4.0"})),
            )
            .mount(&mock_server)
            .await;

        let client = ArrClient::new(&mock_server.uri(), "test-key");
        let resp: serde_json::Value = client.get("system/status").await.unwrap();
        assert_eq!(resp["version"], "4.0");
    }

    #[tokio::test]
    async fn test_api_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/bad"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&mock_server)
            .await;

        let client = ArrClient::new(&mock_server.uri(), "bad-key");
        let result: Result<serde_json::Value, _> = client.get("bad").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("401"));
    }
}
