//! Paean platform API client for credits, usage, and account status.
//!
//! Communicates with the zero-api backend using the stored JWT token
//! or API key for authentication.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::providers::paean_ai::PAEAN_JWT_TOKEN_KEY;

const DEFAULT_API_URL: &str = "https://api.paean.ai";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreditsStatus {
    pub credits: i64,
    #[serde(rename = "totalCredits")]
    pub total_credits: i64,
    #[serde(rename = "subscriptionTier")]
    pub subscription_tier: String,
    #[serde(rename = "nextRecoveryAt")]
    pub next_recovery_at: Option<String>,
    #[serde(rename = "canRecover")]
    pub can_recover: bool,
    #[serde(rename = "recoveryIntervalHours")]
    pub recovery_interval_hours: i64,
    #[serde(rename = "billingPeriod")]
    pub billing_period: Option<String>,
    #[serde(rename = "subscriptionEndDate")]
    pub subscription_end_date: Option<String>,
    #[serde(rename = "paymentSource")]
    pub payment_source: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreditsResponse {
    #[allow(dead_code)]
    pub success: bool,
    pub data: Option<CreditsStatus>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimResult {
    pub success: bool,
    pub message: Option<String>,
    pub data: Option<ClaimData>,
}

#[derive(Debug, Deserialize)]
pub struct ClaimData {
    pub credits: Option<i64>,
    #[serde(rename = "totalCredits")]
    pub total_credits: Option<i64>,
    #[serde(rename = "nextRecoveryAt")]
    pub next_recovery_at: Option<String>,
}

fn get_api_url() -> String {
    Config::global()
        .get_param::<String>("PAEAN_AI_HOST")
        .unwrap_or_else(|_| DEFAULT_API_URL.to_string())
}

fn get_auth_token() -> Result<String> {
    let config = Config::global();
    if let Ok(api_key) = config.get_secret::<String>("PAEAN_AI_API_KEY") {
        return Ok(api_key);
    }
    if let Ok(jwt) = config.get_secret::<String>(PAEAN_JWT_TOKEN_KEY) {
        return Ok(jwt);
    }
    Err(anyhow::anyhow!(
        "Not authenticated. Run `a8e login` or set PAEAN_AI_API_KEY."
    ))
}

fn build_client(token: &str) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))?,
    );
    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .build()?)
}

pub async fn get_credits_status() -> Result<CreditsStatus> {
    let token = get_auth_token()?;
    let client = build_client(&token)?;
    let url = format!("{}/credits/status", get_api_url());

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Failed to fetch credits status (HTTP {}): {}",
            status,
            body
        ));
    }

    let result: CreditsResponse = response.json().await?;
    if let Some(data) = result.data {
        Ok(data)
    } else {
        Err(anyhow::anyhow!(
            "{}",
            result.error.unwrap_or_else(|| "Unknown error".to_string())
        ))
    }
}

pub async fn claim_credits() -> Result<ClaimResult> {
    let token = get_auth_token()?;
    let client = build_client(&token)?;
    let url = format!("{}/credits/claim", get_api_url());

    let response = client.post(&url).send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Failed to claim credits (HTTP {}): {}",
            status,
            body
        ));
    }

    let result: ClaimResult = response.json().await?;
    Ok(result)
}

pub fn is_authenticated() -> bool {
    get_auth_token().is_ok()
}
