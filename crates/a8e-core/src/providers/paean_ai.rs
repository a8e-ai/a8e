use anyhow::Result;
use async_trait::async_trait;
use futures::future::BoxFuture;
use serde_json::Value;

use super::api_client::{ApiClient, AuthMethod};
use super::base::{ConfigKey, MessageStream, Provider, ProviderDef, ProviderMetadata};
use super::errors::ProviderError;
use super::openai_compatible::{handle_status_openai_compat, stream_openai_compat};
use super::retry::ProviderRetry;
use super::utils::{ImageFormat, RequestLog};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::formats::openai::create_request;
use rmcp::model::Tool;

const PAEAN_AI_PROVIDER_NAME: &str = "paean_ai";
pub const PAEAN_AI_DEFAULT_MODEL: &str = "opensota/os-v1";
pub const PAEAN_AI_DEFAULT_FAST_MODEL: &str = "opensota/os-v1-flash";

pub const PAEAN_AI_KNOWN_MODELS: &[&str] = &[
    "opensota/os-v1",
    "opensota/os-v1-pro",
    "opensota/os-v1-mini",
    "opensota/os-v1-flash",
    "opensota/claude-sonnet",
    "anthropic/claude-3-5-sonnet",
    "openai/gpt-4o",
    "openai/gpt-4o-mini",
    "google/gemini-pro",
    "deepseek/deepseek-v3.2-exp",
    "moonshotai/kimi-k2",
    "x-ai/grok-4",
];
pub const PAEAN_AI_DOC_URL: &str = "https://api.paean.ai";

#[derive(serde::Serialize)]
pub struct PaeanAiProvider {
    #[serde(skip)]
    api_client: ApiClient,
    model: ModelConfig,
    supports_streaming: bool,
    #[serde(skip)]
    name: String,
}

impl PaeanAiProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let model = model.with_fast(PAEAN_AI_DEFAULT_FAST_MODEL, PAEAN_AI_PROVIDER_NAME)?;

        let config = crate::config::Config::global();
        let api_key: String = config.get_secret("PAEAN_AI_API_KEY")?;
        let host: String = config
            .get_param("PAEAN_AI_HOST")
            .unwrap_or_else(|_| "https://api.paean.ai".to_string());

        let auth = AuthMethod::BearerToken(api_key);
        let api_client = ApiClient::new(host, auth)?;

        Ok(Self {
            api_client,
            model,
            supports_streaming: true,
            name: PAEAN_AI_PROVIDER_NAME.to_string(),
        })
    }
}

impl ProviderDef for PaeanAiProvider {
    type Provider = Self;

    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            PAEAN_AI_PROVIDER_NAME,
            "Paean AI",
            "AI gateway with multi-provider model routing",
            PAEAN_AI_DEFAULT_MODEL,
            PAEAN_AI_KNOWN_MODELS.to_vec(),
            PAEAN_AI_DOC_URL,
            vec![
                ConfigKey::new("PAEAN_AI_API_KEY", true, true, None, true),
                ConfigKey::new(
                    "PAEAN_AI_HOST",
                    false,
                    false,
                    Some("https://api.paean.ai"),
                    false,
                ),
            ],
        )
        .with_unlisted_models()
    }

    fn from_env(
        model: ModelConfig,
        _extensions: Vec<crate::config::ExtensionConfig>,
    ) -> BoxFuture<'static, Result<Self::Provider>> {
        Box::pin(Self::from_env(model))
    }
}

#[async_trait]
impl Provider for PaeanAiProvider {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        let response = self
            .api_client
            .request(None, "v1/models")
            .response_get()
            .await
            .map_err(|e| {
                ProviderError::RequestFailed(format!("Failed to fetch models from Paean AI: {}", e))
            })?;

        let json: Value = response.json().await.map_err(|e| {
            ProviderError::RequestFailed(format!(
                "Failed to parse Paean AI response as JSON: {}",
                e
            ))
        })?;

        if let Some(err_obj) = json.get("error") {
            let msg = err_obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(ProviderError::RequestFailed(format!(
                "Paean AI returned an error: {}",
                msg
            )));
        }

        let data = json.get("data").and_then(|v| v.as_array()).ok_or_else(|| {
            ProviderError::UsageError("Missing data field in JSON response".into())
        })?;

        let mut models: Vec<String> = data
            .iter()
            .filter_map(|model| {
                let id = model.get("id").and_then(|v| v.as_str())?;
                Some(id.to_string())
            })
            .collect();

        models.sort();
        Ok(models)
    }

    async fn stream(
        &self,
        model_config: &ModelConfig,
        session_id: &str,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let mut payload = create_request(
            model_config,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            true,
        )?;

        if !session_id.is_empty() {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("user".to_string(), Value::String(session_id.to_string()));
            }
        }

        let mut log = RequestLog::start(model_config, &payload)?;

        let response = self
            .with_retry(|| async {
                let resp = self
                    .api_client
                    .response_post(Some(session_id), "v1/chat/completions", &payload)
                    .await?;
                handle_status_openai_compat(resp).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        stream_openai_compat(response, log)
    }
}
