use a8e_core::agents::{Agent, AgentEvent, SessionConfig};
use a8e_core::conversation::message::Message;
use a8e_core::session::session_manager::SessionType;
use anyhow::Result;
use futures::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const CHANNEL_VERSION: &str = "0.1.0";
const BOT_TYPE: &str = "3";
const MSG_TYPE_USER: i64 = 1;
const MSG_TYPE_BOT: i64 = 2;
const MSG_STATE_FINISH: i64 = 2;
const MSG_ITEM_TEXT: i64 = 1;
const MSG_ITEM_VOICE: i64 = 3;
const MAX_MSG_CHUNK: usize = 2048;
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const BACKOFF_DELAY_SECS: u64 = 30;
const RETRY_DELAY_SECS: u64 = 2;

#[derive(Deserialize)]
struct TextItem {
    text: Option<String>,
}
#[derive(Deserialize)]
struct VoiceItem {
    text: Option<String>,
}
#[derive(Deserialize)]
struct RefMsg {
    title: Option<String>,
}
#[derive(Deserialize)]
struct MessageItem {
    #[serde(rename = "type")]
    item_type: Option<i64>,
    text_item: Option<TextItem>,
    voice_item: Option<VoiceItem>,
    ref_msg: Option<RefMsg>,
}
#[derive(Deserialize)]
struct WeixinMessage {
    from_user_id: Option<String>,
    message_type: Option<i64>,
    item_list: Option<Vec<MessageItem>>,
    context_token: Option<String>,
}
#[derive(Deserialize)]
struct GetUpdatesResp {
    ret: Option<i64>,
    errcode: Option<i64>,
    msgs: Option<Vec<WeixinMessage>>,
    get_updates_buf: Option<String>,
}
#[derive(Deserialize)]
struct QRCodeResponse {
    qrcode: String,
    qrcode_img_content: String,
}
#[derive(Deserialize)]
struct QRStatusResponse {
    status: String,
    bot_token: Option<String>,
    ilink_bot_id: Option<String>,
    baseurl: Option<String>,
}

fn random_uin() -> String {
    use base64::Engine;
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    base64::engine::general_purpose::STANDARD.encode(t.as_nanos().to_string())
}

fn norm(base: &str) -> String {
    if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{}/", base)
    }
}

fn build_client() -> reqwest::Client {
    reqwest::Client::new()
}

fn auth_headers(token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Content-Type", "application/json".into()),
        ("AuthorizationType", "ilink_bot_token".into()),
        ("X-WECHAT-UIN", random_uin()),
        ("Authorization", format!("Bearer {}", token.trim())),
    ]
}

fn extract_text(msg: &WeixinMessage) -> Option<String> {
    for item in msg.item_list.as_deref()? {
        if item.item_type == Some(MSG_ITEM_TEXT) {
            if let Some(ref ti) = item.text_item {
                if let Some(ref text) = ti.text {
                    if let Some(ref rm) = item.ref_msg {
                        if let Some(ref title) = rm.title {
                            return Some(format!("[Quote: {}]\n{}", title, text));
                        }
                    }
                    return Some(text.clone());
                }
            }
        }
        if item.item_type == Some(MSG_ITEM_VOICE) {
            if let Some(ref vi) = item.voice_item {
                if let Some(ref text) = vi.text {
                    return Some(text.clone());
                }
            }
        }
    }
    None
}

async fn get_updates(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    buf: &str,
) -> Result<GetUpdatesResp> {
    let url = format!("{}ilink/bot/getupdates", norm(base_url));
    let body = serde_json::json!({
        "get_updates_buf": buf,
        "base_info": { "channel_version": CHANNEL_VERSION },
    });
    let mut req = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(35))
        .json(&body);
    for (k, v) in auth_headers(token) {
        req = req.header(k, &v);
    }
    Ok(req.send().await?.json().await?)
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
        "a8e:{}",
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
    let mut req = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(15))
        .json(&body);
    for (k, v) in auth_headers(token) {
        req = req.header(k, &v);
    }
    req.send().await?;
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

fn get_provider_and_model() -> (String, String) {
    let config = a8e_core::config::Config::global();
    let provider = config
        .get_a8e_provider()
        .unwrap_or_else(|_| "anthropic".into());
    let model = config
        .get_a8e_model()
        .unwrap_or_else(|_| "claude-sonnet-4-20250514".into());
    (provider, model)
}

async fn create_agent(provider_name: &str, model: &str) -> Result<Agent> {
    let model_config =
        a8e_core::model::ModelConfig::new(model)?.with_canonical_limits(provider_name);
    let agent = Agent::new();

    let session_manager = agent.config.session_manager.clone();
    let init_session = session_manager
        .create_session(
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            "WeChat Agent Init".to_string(),
            SessionType::Hidden,
        )
        .await?;

    let enabled_configs = a8e_core::config::resolve_extensions_for_new_session(None, None);
    for config in &enabled_configs {
        if let Err(e) = agent.add_extension(config.clone(), &init_session.id).await {
            tracing::warn!("Failed to load extension {}: {}", config.name(), e);
        }
    }

    let provider =
        a8e_core::providers::create(provider_name, model_config, enabled_configs).await?;
    agent.update_provider(provider, &init_session.id).await?;
    Ok(agent)
}

pub async fn handle_wechat_setup() -> Result<()> {
    let client = build_client();
    let base_url = DEFAULT_BASE_URL;

    println!("Fetching WeChat login QR code...\n");
    let url = format!(
        "{}ilink/bot/get_bot_qrcode?bot_type={}",
        norm(base_url),
        BOT_TYPE
    );
    let qr: QRCodeResponse = client.get(&url).send().await?.json().await?;

    qr2term::print_qr(&qr.qrcode_img_content).ok();
    println!("\nScan the QR code with WeChat...\n");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(480);
    let mut scanned = false;
    while std::time::Instant::now() < deadline {
        let status_url = format!(
            "{}ilink/bot/get_qrcode_status?qrcode={}",
            norm(base_url),
            urlencoding::encode(&qr.qrcode)
        );
        let status: QRStatusResponse = client
            .get(&status_url)
            .header("iLink-App-ClientVersion", "1")
            .timeout(std::time::Duration::from_secs(35))
            .send()
            .await?
            .json()
            .await?;

        match status.status.as_str() {
            "wait" => eprint!("."),
            "scaned" => {
                if !scanned {
                    eprintln!("\nScanned! Confirm on phone...");
                    scanned = true;
                }
            }
            "expired" => {
                eprintln!("\nQR code expired. Run setup again.");
                std::process::exit(1);
            }
            "confirmed" => {
                let token = status
                    .bot_token
                    .ok_or_else(|| anyhow::anyhow!("Missing bot_token"))?;
                let account_id = status
                    .ilink_bot_id
                    .ok_or_else(|| anyhow::anyhow!("Missing ilink_bot_id"))?;
                let base = status.baseurl.unwrap_or_else(|| base_url.to_string());

                println!("\n{} WeChat connected!", console::style("✓").green().bold());
                println!("  Account: {}", account_id);
                println!();
                println!(
                    "{} Add these to your environment (e.g. ~/.zshrc or .env):",
                    console::style("→").cyan()
                );
                println!();
                println!("  export A8E_WECHAT_TOKEN={}", token);
                println!("  export A8E_WECHAT_BASE_URL={}", base);
                println!("  export A8E_WECHAT_ACCOUNT_ID={}", account_id);
                println!();
                println!(
                    "{} Then start the WeChat channel:",
                    console::style("→").cyan()
                );
                println!("  a8e wechat start");
                return Ok(());
            }
            _ => {}
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    eprintln!("\nLogin timed out.");
    std::process::exit(1);
}

pub async fn handle_wechat_start() -> Result<()> {
    let token = std::env::var("A8E_WECHAT_TOKEN")
        .map_err(|_| anyhow::anyhow!("A8E_WECHAT_TOKEN not set. Run `a8e wechat setup` first."))?;
    let base_url =
        std::env::var("A8E_WECHAT_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
    let account_id =
        std::env::var("A8E_WECHAT_ACCOUNT_ID").unwrap_or_else(|_| "unknown".to_string());

    crate::logging::setup_logging(Some("a8e-wechat"))?;

    let (provider_name, model) = get_provider_and_model();
    let agent = Arc::new(create_agent(&provider_name, &model).await?);

    println!(
        "\n{} Starting a8e WeChat channel",
        console::style("∞").magenta().bold()
    );
    println!(
        "   {} WeChat channel connected",
        console::style("💬").bold()
    );
    println!("   Provider: {} | Model: {}", provider_name, model);
    println!("   Account: {}", account_id);
    println!(
        "   Working directory: {}",
        std::env::current_dir()?.display()
    );
    println!("   Press Ctrl+C to stop\n");

    let client = build_client();
    let mut update_buf = String::new();
    let mut failures: u32 = 0;
    let mut ctx_cache: HashMap<String, String> = HashMap::new();
    let mut session_cache: HashMap<String, String> = HashMap::new();

    loop {
        match get_updates(&client, &base_url, &token, &update_buf).await {
            Err(e) => {
                failures += 1;
                eprintln!("[wechat] Poll error: {e}");
                let delay = if failures >= MAX_CONSECUTIVE_FAILURES {
                    failures = 0;
                    BACKOFF_DELAY_SECS
                } else {
                    RETRY_DELAY_SECS
                };
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
            Ok(resp) => {
                let is_err =
                    resp.ret.is_some_and(|r| r != 0) || resp.errcode.is_some_and(|e| e != 0);
                if is_err {
                    failures += 1;
                    eprintln!(
                        "[wechat] getUpdates error: ret={:?} errcode={:?}",
                        resp.ret, resp.errcode
                    );
                    let delay = if failures >= MAX_CONSECUTIVE_FAILURES {
                        failures = 0;
                        BACKOFF_DELAY_SECS
                    } else {
                        RETRY_DELAY_SECS
                    };
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue;
                }
                failures = 0;
                if let Some(buf) = resp.get_updates_buf {
                    update_buf = buf;
                }

                for msg in resp.msgs.unwrap_or_default() {
                    if msg.message_type != Some(MSG_TYPE_USER) {
                        continue;
                    }
                    let text = match extract_text(&msg) {
                        Some(t) if !t.is_empty() => t,
                        _ => continue,
                    };
                    let sender = msg.from_user_id.as_deref().unwrap_or("unknown");
                    if let Some(ct) = &msg.context_token {
                        ctx_cache.insert(sender.to_string(), ct.clone());
                        save_contact(sender, ct);
                    }

                    let name = sender.split('@').next().unwrap_or(sender);
                    let preview_end = text.floor_char_boundary(80);
                    let preview: &str = text.get(..preview_end).unwrap_or(&text);
                    eprintln!(
                        "[wechat] 💬 ← {}: {}{}",
                        name,
                        preview,
                        if text.len() > 80 { "..." } else { "" }
                    );

                    let session_manager = agent.config.session_manager.clone();
                    let session_id = if let Some(existing_id) = session_cache.get(sender) {
                        match session_manager.get_session(existing_id, false).await {
                            Ok(_) => existing_id.clone(),
                            Err(_) => {
                                let s = session_manager
                                    .create_session(
                                        std::env::current_dir()
                                            .unwrap_or_else(|_| std::path::PathBuf::from(".")),
                                        format!("WeChat: {}", name),
                                        SessionType::User,
                                    )
                                    .await?;
                                session_cache.insert(sender.to_string(), s.id.clone());
                                s.id
                            }
                        }
                    } else {
                        let s = session_manager
                            .create_session(
                                std::env::current_dir()
                                    .unwrap_or_else(|_| std::path::PathBuf::from(".")),
                                format!("WeChat: {}", name),
                                SessionType::User,
                            )
                            .await?;
                        session_cache.insert(sender.to_string(), s.id.clone());
                        s.id
                    };

                    let user_message = Message::user().with_text(&text);
                    let session_config = SessionConfig {
                        id: session_id,
                        schedule_id: None,
                        max_turns: None,
                        retry_config: None,
                    };

                    let mut full_content = String::new();
                    match agent.reply(user_message, session_config, None).await {
                        Ok(mut stream) => {
                            while let Some(result) = stream.next().await {
                                match result {
                                    Ok(AgentEvent::Message(msg)) => {
                                        use a8e_core::conversation::message::MessageContent;
                                        for content in &msg.content {
                                            if let MessageContent::Text(t) = content {
                                                full_content.push_str(&t.text);
                                            }
                                        }
                                    }
                                    Ok(AgentEvent::McpNotification((server, _))) => {
                                        eprintln!("[wechat]   🔧 MCP: {server}");
                                    }
                                    Ok(AgentEvent::ModelChange { model: m, mode }) => {
                                        eprintln!("[wechat]   ⚙ Model: {m} ({mode})");
                                    }
                                    Ok(_) => {}
                                    Err(e) => {
                                        eprintln!("[wechat]   ✗ Agent error: {e}");
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[wechat]   ✗ Error: {e}");
                            full_content = format!("Error: {e}");
                        }
                    }

                    if !full_content.is_empty() {
                        let resp_end = full_content.floor_char_boundary(100);
                        let resp_preview: &str =
                            full_content.get(..resp_end).unwrap_or(&full_content);
                        eprintln!(
                            "[wechat] 💬 → {}: {}{}",
                            name,
                            resp_preview,
                            if full_content.len() > 100 { "..." } else { "" }
                        );

                        if let Some(ctx) = ctx_cache.get(sender) {
                            for chunk in chunk_text(&full_content, MAX_MSG_CHUNK) {
                                if let Err(e) =
                                    send_text(&client, &base_url, &token, sender, chunk, ctx).await
                                {
                                    eprintln!("[wechat] Send error: {e}");
                                }
                            }
                            eprintln!("[wechat] ✓ Reply sent to {name}");
                        }
                    }
                }
            }
        }
    }
}

pub async fn handle_wechat_status() -> Result<()> {
    match std::env::var("A8E_WECHAT_TOKEN") {
        Ok(_) => {
            let account = std::env::var("A8E_WECHAT_ACCOUNT_ID").unwrap_or_else(|_| "N/A".into());
            let base = std::env::var("A8E_WECHAT_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
            println!("{} WeChat: Configured", console::style("✓").green().bold());
            println!("  Account: {}", account);
            println!("  Base URL: {}", base);
        }
        Err(_) => {
            println!("{} WeChat: Not configured", console::style("✗").red());
            println!("  Run `a8e wechat setup` to authenticate.");
        }
    }
    Ok(())
}

// ── Contact persistence ──────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct WechatContact {
    user_id: String,
    context_token: String,
    last_seen: String,
    display_name: Option<String>,
}

fn contacts_file_path() -> std::path::PathBuf {
    a8e_core::config::paths::Paths::data_dir().join("wechat_contacts.json")
}

fn load_contacts() -> Vec<WechatContact> {
    let path = contacts_file_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_contact(user_id: &str, context_token: &str) {
    let mut contacts = load_contacts();
    let display_name = user_id.split('@').next().unwrap_or(user_id).to_string();
    let entry = WechatContact {
        user_id: user_id.to_string(),
        context_token: context_token.to_string(),
        last_seen: chrono::Utc::now().to_rfc3339(),
        display_name: Some(display_name),
    };
    if let Some(idx) = contacts.iter().position(|c| c.user_id == user_id) {
        contacts[idx] = entry;
    } else {
        contacts.push(entry);
    }
    let path = contacts_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&contacts) {
        let _ = std::fs::write(&path, json);
    }
}

fn get_contact_token(user_id_or_name: &str) -> Option<String> {
    let contacts = load_contacts();
    contacts
        .iter()
        .find(|c| {
            c.user_id == user_id_or_name || c.display_name.as_deref() == Some(user_id_or_name)
        })
        .map(|c| c.context_token.clone())
}

pub async fn handle_wechat_send_message(to: &str, text: &str) -> Result<()> {
    let token = std::env::var("A8E_WECHAT_TOKEN")
        .map_err(|_| anyhow::anyhow!("A8E_WECHAT_TOKEN not set. Run `a8e wechat setup` first."))?;
    let base_url =
        std::env::var("A8E_WECHAT_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());

    let context_token = get_contact_token(to).ok_or_else(|| {
        anyhow::anyhow!(
            "No context token found for \"{}\". The user must message the bot first.\n\
             Run `a8e wechat contacts` to see known contacts.",
            to
        )
    })?;

    let client = build_client();
    for chunk in chunk_text(text, MAX_MSG_CHUNK) {
        send_text(&client, &base_url, &token, to, chunk, &context_token).await?;
    }

    println!(
        "{} Message sent to {}",
        console::style("✓").green().bold(),
        to
    );
    Ok(())
}

pub async fn handle_wechat_contacts() -> Result<()> {
    let contacts = load_contacts();
    if contacts.is_empty() {
        println!("No contacts yet. Start the WeChat channel and receive messages first.");
        return Ok(());
    }
    println!(
        "{}\n",
        console::style(format!("Known WeChat contacts ({}):", contacts.len())).bold()
    );
    for c in &contacts {
        let name = c.display_name.as_deref().unwrap_or(&c.user_id);
        println!("  {}", console::style(name).cyan());
        println!("    User ID:   {}", c.user_id);
        println!("    Last seen: {}\n", c.last_seen);
    }
    Ok(())
}
