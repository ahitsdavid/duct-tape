use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {0}")]
    Api(String),
}

#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn complete(&self, messages: &[Message]) -> Result<String, LlmError>;
    async fn health_check(&self) -> Result<bool, LlmError>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

pub struct HttpLlmBackend {
    client: Client,
    api_url: String,
    api_key: Option<String>,
}

impl HttpLlmBackend {
    pub fn new(api_url: &str, api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_url: api_url.trim_end_matches('/').to_string(),
            api_key,
        }
    }
}

#[async_trait]
impl LlmBackend for HttpLlmBackend {
    async fn complete(&self, messages: &[Message]) -> Result<String, LlmError> {
        let body = serde_json::json!({
            "messages": messages,
            "stream": false,
        });

        let mut req = self.client.post(format!("{}/v1/messages", self.api_url));
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!("{status}: {text}")));
        }

        let json: serde_json::Value = resp.json().await?;

        // Try common response formats
        if let Some(content) = json["content"][0]["text"].as_str() {
            return Ok(content.to_string());
        }
        if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
            return Ok(content.to_string());
        }
        if let Some(content) = json["response"].as_str() {
            return Ok(content.to_string());
        }

        Err(LlmError::Api(format!(
            "Could not parse response: {}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        )))
    }

    async fn health_check(&self) -> Result<bool, LlmError> {
        let resp = self
            .client
            .get(format!("{}/health", self.api_url))
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_complete_anthropic_format() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "Hello! How can I help?"}]
            })))
            .mount(&mock_server)
            .await;

        let backend = HttpLlmBackend::new(&mock_server.uri(), None);
        let messages = vec![Message { role: "user".into(), content: "Hi".into() }];
        let result = backend.complete(&messages).await.unwrap();
        assert_eq!(result, "Hello! How can I help?");
    }

    #[tokio::test]
    async fn test_complete_openai_format() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "Hello from OpenAI format"}}]
            })))
            .mount(&mock_server)
            .await;

        let backend = HttpLlmBackend::new(&mock_server.uri(), None);
        let messages = vec![Message { role: "user".into(), content: "Hi".into() }];
        let result = backend.complete(&messages).await.unwrap();
        assert_eq!(result, "Hello from OpenAI format");
    }

    #[tokio::test]
    async fn test_health_check_healthy() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let backend = HttpLlmBackend::new(&mock_server.uri(), None);
        assert!(backend.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn test_health_check_unhealthy() {
        let backend = HttpLlmBackend::new("http://127.0.0.1:1", None);
        assert!(!backend.health_check().await.unwrap());
    }
}
