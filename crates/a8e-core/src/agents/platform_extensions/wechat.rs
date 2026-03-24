use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use anyhow::Result;
use async_trait::async_trait;
use rmcp::model::{
    CallToolResult, Content, Implementation, InitializeResult, JsonObject, ListToolsResult,
    ProtocolVersion, ServerCapabilities, Tool, ToolAnnotations, ToolsCapability,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "wechat";

const CHANNEL_VERSION: &str = "0.1.0";
const MSG_TYPE_BOT: i64 = 2;
const MSG_STATE_FINISH: i64 = 2;
const MSG_ITEM_TEXT: i64 = 1;
const MAX_MSG_CHUNK: usize = 2048;

// ── Tool parameter schemas ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct WechatSendParams {
    /// Plain-text message to send. Keep concise for WeChat chat format.
    text: String,
    /// Target user ID or display name. Omit to send to the most recent contact.
    #[serde(default)]
    to: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct WechatContactsParams {}

// ── Contact persistence (shared with a8e-cli wechat commands) ───────────

#[derive(Serialize, Deserialize, Clone)]
struct WechatContact {
    user_id: String,
    context_token: String,
    last_seen: String,
    display_name: Option<String>,
}

fn contacts_file_path() -> std::path::PathBuf {
    crate::config::paths::Paths::data_dir().join("wechat_contacts.json")
}

fn load_contacts() -> Vec<WechatContact> {
    let path = contacts_file_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

// ── WeChat API helpers ──────────────────────────────────────────────────

fn random_uin() -> String {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Simple base64 of timestamp nanos, matching the a8e-cli convention
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(t.as_nanos().to_string())
}

fn norm(base: &str) -> String {
    if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    }
}

async fn send_text(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    to: &str,
    text: &str,
    ctx_token: &str,
) -> Result<()> {
    let url = format!("{}ilink/bot/sendmessage", norm(base_url));
    let client_id = format!(
        "a8e-ext:{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let body = serde_json::json!({
        "msg": {
            "from_user_id": "", "to_user_id": to, "client_id": client_id,
            "message_type": MSG_TYPE_BOT, "message_state": MSG_STATE_FINISH,
            "item_list": [{ "type": MSG_ITEM_TEXT, "text_item": { "text": text } }],
            "context_token": ctx_token,
        },
        "base_info": { "channel_version": CHANNEL_VERSION },
    });
    let resp = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(15))
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("X-WECHAT-UIN", random_uin())
        .header("Authorization", format!("Bearer {}", token.trim()))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("WeChat API error {status}: {text}");
    }
    Ok(())
}

fn chunk_text(s: &str, max: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let end = s.floor_char_boundary((start + max).min(s.len()));
        let end = if end < s.len() {
            s.get(start..end)
                .and_then(|sub| sub.rfind(char::is_whitespace))
                .map_or(end, |p| start + p + 1)
        } else {
            end
        };
        if let Some(chunk) = s.get(start..end) {
            chunks.push(chunk);
        }
        start = end;
    }
    chunks
}

// ── Platform extension ──────────────────────────────────────────────────

pub struct WechatClient {
    info: InitializeResult,
    http: reqwest::Client,
}

impl WechatClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                tasks: None,
                resources: None,
                extensions: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
            },
            server_info: Implementation {
                name: EXTENSION_NAME.to_string(),
                description: None,
                title: Some("WeChat".to_string()),
                version: "1.0.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Send proactive messages to WeChat contacts. \
                 Use wechat_send to push important notifications (task completions, \
                 loop task results, summaries) to the connected WeChat user. \
                 Use wechat_contacts to check who is available. \
                 The context_token is valid for 24 hours after the user's last WeChat message."
                    .to_string(),
            ),
        };
        Ok(Self {
            info,
            http: reqwest::Client::new(),
        })
    }

    fn get_tools() -> Vec<Tool> {
        let send_schema = schemars::schema_for!(WechatSendParams);
        let send_value =
            serde_json::to_value(send_schema).expect("Failed to serialize WechatSendParams schema");

        let contacts_schema = schemars::schema_for!(WechatContactsParams);
        let contacts_value = serde_json::to_value(contacts_schema)
            .expect("Failed to serialize WechatContactsParams schema");

        vec![
            Tool::new(
                "wechat_send".to_string(),
                "Send a message to a WeChat contact. \
                 If no 'to' is specified, sends to the most recently active contact. \
                 Requires A8E_WECHAT_TOKEN to be configured (via `a8e wechat setup`)."
                    .to_string(),
                send_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Send WeChat Message".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(true),
            }),
            Tool::new(
                "wechat_contacts".to_string(),
                "List known WeChat contacts who have previously messaged this bot. \
                 Returns user IDs, display names, and last-seen timestamps."
                    .to_string(),
                contacts_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("List WeChat Contacts".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        ]
    }

    async fn handle_send(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.as_ref().ok_or("Missing arguments")?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: text")?;

        let token = std::env::var("A8E_WECHAT_TOKEN").map_err(|_| {
            "A8E_WECHAT_TOKEN not set. Run `a8e wechat setup` to connect a WeChat account."
                .to_string()
        })?;
        let base_url = std::env::var("A8E_WECHAT_BASE_URL")
            .unwrap_or_else(|_| "https://ilinkai.weixin.qq.com".to_string());

        let contacts = load_contacts();
        if contacts.is_empty() {
            return Err(
                "No known WeChat contacts. A user must send a message via WeChat first."
                    .to_string(),
            );
        }

        let to_arg = args.get("to").and_then(|v| v.as_str());

        let (target_user_id, ctx_token, display_name) = if let Some(to) = to_arg {
            let contact = contacts
                .iter()
                .find(|c| c.user_id == to || c.display_name.as_deref() == Some(to))
                .ok_or_else(|| {
                    let names: Vec<&str> = contacts
                        .iter()
                        .map(|c| c.display_name.as_deref().unwrap_or(&c.user_id))
                        .collect();
                    format!(
                        "No context token for \"{to}\". Available contacts: {}",
                        names.join(", ")
                    )
                })?;
            (
                contact.user_id.as_str(),
                contact.context_token.as_str(),
                contact.display_name.as_deref().unwrap_or(&contact.user_id),
            )
        } else {
            let recent = contacts.iter().max_by_key(|c| c.last_seen.clone()).unwrap(); // safe: contacts is non-empty
            (
                recent.user_id.as_str(),
                recent.context_token.as_str(),
                recent.display_name.as_deref().unwrap_or(&recent.user_id),
            )
        };

        let chunks = chunk_text(text, MAX_MSG_CHUNK);
        let chunk_count = chunks.len();
        for chunk in chunks {
            send_text(
                &self.http,
                &base_url,
                &token,
                target_user_id,
                chunk,
                ctx_token,
            )
            .await
            .map_err(|e| format!("WeChat send failed: {e}"))?;
        }

        Ok(vec![Content::text(format!(
            "Sent {chunk_count} message(s) to {display_name}"
        ))])
    }

    fn handle_contacts(&self) -> Result<Vec<Content>, String> {
        let contacts = load_contacts();
        let has_token = std::env::var("A8E_WECHAT_TOKEN").is_ok();

        let result = serde_json::json!({
            "authenticated": has_token,
            "contacts": contacts.iter().map(|c| serde_json::json!({
                "userId": c.user_id,
                "displayName": c.display_name.as_deref().unwrap_or_else(|| c.user_id.split('@').next().unwrap_or(&c.user_id)),
                "lastSeen": c.last_seen,
            })).collect::<Vec<_>>(),
            "count": contacts.len(),
        });

        Ok(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| "[]".to_string()),
        )])
    }
}

#[async_trait]
impl McpClientTrait for WechatClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _session_id: &str,
        name: &str,
        arguments: Option<JsonObject>,
        _working_dir: Option<&str>,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let result = match name {
            "wechat_send" => self.handle_send(arguments).await,
            "wechat_contacts" => self.handle_contacts(),
            _ => Err(format!("Unknown tool: {name}")),
        };

        match result {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {error}"
            ))])),
        }
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
