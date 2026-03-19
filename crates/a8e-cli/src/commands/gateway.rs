use a8e_core::agents::{Agent, AgentEvent, SessionConfig};
use a8e_core::conversation::message::Message;
use a8e_core::session::session_manager::SessionType;
use anyhow::Result;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, net::ToSocketAddrs, sync::Arc};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
struct GatewayState {
    agent: Arc<Agent>,
    auth_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatRequest {
    message: String,
    #[serde(default)]
    conversation_id: Option<String>,
}

#[derive(Serialize)]
struct GatewayEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(flatten)]
    data: serde_json::Value,
}

fn check_auth(auth_token: &Option<String>, headers: &http::HeaderMap) -> bool {
    let Some(ref expected) = auth_token else {
        return true;
    };

    if let Some(header) = headers.get("authorization") {
        if let Ok(val) = header.to_str() {
            if let Some(token) = val.strip_prefix("Bearer ") {
                return token == expected.as_str();
            }
        }
    }

    false
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
            "Gateway Agent Init".to_string(),
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

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "a8e-gateway",
        "protocol": "claw"
    }))
}

async fn chat(
    State(state): State<GatewayState>,
    headers: http::HeaderMap,
    Json(req): Json<ChatRequest>,
) -> Result<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    if !check_auth(&state.auth_token, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let (tx, rx) = mpsc::channel::<GatewayEvent>(64);
    let agent = state.agent.clone();
    let message = req.message;
    let conv_id = req.conversation_id;

    tokio::spawn(async move {
        if let Err(e) = process_chat(agent, message, conv_id, tx.clone()).await {
            let _ = tx
                .send(GatewayEvent {
                    event_type: "error".into(),
                    data: serde_json::json!({ "error": e.to_string() }),
                })
                .await;
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx).map(|event| {
        Ok(Event::default().data(serde_json::to_string(&event).unwrap_or_default()))
    })))
}

async fn process_chat(
    agent: Arc<Agent>,
    message: String,
    conversation_id: Option<String>,
    tx: mpsc::Sender<GatewayEvent>,
) -> Result<()> {
    let session_manager = agent.config.session_manager.clone();

    let session = if let Some(ref cid) = conversation_id {
        match session_manager.get_session(cid, false).await {
            Ok(s) => s,
            Err(_) => {
                session_manager
                    .create_session(
                        std::env::current_dir()?,
                        "Gateway session".to_string(),
                        SessionType::User,
                    )
                    .await?
            }
        }
    } else {
        session_manager
            .create_session(
                std::env::current_dir()?,
                "Gateway session".to_string(),
                SessionType::User,
            )
            .await?
    };

    let _ = tx
        .send(GatewayEvent {
            event_type: "start".into(),
            data: serde_json::json!({ "conversationId": session.id }),
        })
        .await;

    let user_message = Message::user().with_text(&message);

    let session_config = SessionConfig {
        id: session.id.clone(),
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
                            match content {
                                MessageContent::Text(text) => {
                                    full_content.push_str(&text.text);
                                    let _ = tx
                                        .send(GatewayEvent {
                                            event_type: "content".into(),
                                            data: serde_json::json!({ "text": text.text }),
                                        })
                                        .await;
                                }
                                MessageContent::ToolRequest(req) => {
                                    if let Ok(tool_call) = &req.tool_call {
                                        let _ = tx
                                            .send(GatewayEvent {
                                                event_type: "tool_call".into(),
                                                data: serde_json::json!({
                                                    "name": tool_call.name.to_string(),
                                                    "id": req.id,
                                                }),
                                            })
                                            .await;
                                    }
                                }
                                MessageContent::ToolResponse(resp) => {
                                    let _ = tx
                                        .send(GatewayEvent {
                                            event_type: "tool_result".into(),
                                            data: serde_json::json!({
                                                "name": resp.id,
                                                "status": "completed",
                                            }),
                                        })
                                        .await;
                                }
                                MessageContent::ToolConfirmationRequest(confirmation) => {
                                    agent
                                        .handle_confirmation(
                                            confirmation.id.clone(),
                                            a8e_core::permission::PermissionConfirmation {
                                                principal_type:
                                                    a8e_core::permission::permission_confirmation::PrincipalType::Tool,
                                                permission: a8e_core::permission::Permission::AllowOnce,
                                            },
                                        )
                                        .await;
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(AgentEvent::HistoryReplaced(_)) => {}
                    Ok(AgentEvent::McpNotification(_)) => {}
                    Ok(AgentEvent::ModelChange { .. }) => {}
                    Err(e) => {
                        let _ = tx
                            .send(GatewayEvent {
                                event_type: "error".into(),
                                data: serde_json::json!({ "error": e.to_string() }),
                            })
                            .await;
                        break;
                    }
                }
            }
        }
        Err(e) => {
            let _ = tx
                .send(GatewayEvent {
                    event_type: "error".into(),
                    data: serde_json::json!({ "error": e.to_string() }),
                })
                .await;
        }
    }

    let _ = tx
        .send(GatewayEvent {
            event_type: "done".into(),
            data: serde_json::json!({
                "content": full_content,
                "conversationId": session.id,
            }),
        })
        .await;

    Ok(())
}

#[derive(Deserialize)]
struct MessagesQuery {
    #[serde(rename = "conversationId")]
    conversation_id: String,
}

async fn conversations(
    State(state): State<GatewayState>,
    headers: http::HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !check_auth(&state.auth_token, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    match state.agent.config.session_manager.list_sessions().await {
        Ok(sessions) => {
            let list: Vec<serde_json::Value> = sessions
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "title": s.name,
                        "createdAt": s.created_at.to_rfc3339(),
                    })
                })
                .collect();
            Ok(Json(serde_json::json!(list)))
        }
        Err(e) => Ok(Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

async fn messages(
    State(state): State<GatewayState>,
    headers: http::HeaderMap,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !check_auth(&state.auth_token, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    match state
        .agent
        .config
        .session_manager
        .get_session(&q.conversation_id, true)
        .await
    {
        Ok(session) => {
            let msgs = session.conversation.unwrap_or_default();
            Ok(Json(serde_json::json!(msgs.messages())))
        }
        Err(e) => Ok(Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

fn build_router(state: GatewayState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/chat", post(chat))
        .route("/api/conversations", get(conversations))
        .route("/api/messages", get(messages))
        .route("/api/health", get(health_check))
        .layer(cors)
        .with_state(state)
}

pub async fn handle_gateway(
    port: u16,
    host: String,
    auth_token: Option<String>,
    no_auth: bool,
) -> Result<()> {
    if !is_loopback_address(&host) && auth_token.is_none() && !no_auth {
        eprintln!(
            "Error: --auth-token is required when the server is exposed on the network ({}).",
            host
        );
        eprintln!("Use --auth-token <TOKEN> or bind to a local address (e.g., localhost).");
        eprintln!("To skip this check, use --no-auth (unsafe).");
        std::process::exit(1);
    }

    crate::logging::setup_logging(Some("a8e-gateway"))?;

    let (provider_name, model) = get_provider_and_model();
    let agent = create_agent(&provider_name, &model).await?;

    let state = GatewayState {
        agent: Arc::new(agent),
        auth_token,
    };

    let app = build_router(state);

    let addr = (host.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve address: {}", host))?;

    println!(
        "\n{} Starting a8e gateway (claw protocol)",
        console::style("∞").magenta().bold()
    );
    println!("   Provider: {} | Model: {}", provider_name, model);
    println!(
        "   Working directory: {}",
        std::env::current_dir()?.display()
    );
    println!("   Gateway: http://{}", addr);
    println!("   Endpoints:");
    println!("     POST /api/chat          SSE-streamed agent interaction");
    println!("     GET  /api/conversations  List conversations");
    println!("     GET  /api/messages       Get messages for a conversation");
    println!("     GET  /api/health         Health check");
    println!();
    println!("   Compatible with: anyclaw, zeroclaw, 0claw, and any claw-protocol client");
    println!("   Press Ctrl+C to stop\n");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn is_loopback_address(host: &str) -> bool {
    (host, 0)
        .to_socket_addrs()
        .map(|mut addrs| addrs.any(|addr| addr.ip().is_loopback()))
        .unwrap_or(false)
}
