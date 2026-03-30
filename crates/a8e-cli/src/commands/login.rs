use a8e_core::config::Config;
use anyhow::Result;
use axum::extract::{Query, State};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use console::style;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

pub const PAEAN_JWT_TOKEN_KEY: &str = "PAEAN_JWT_TOKEN";
pub const PAEAN_USER_EMAIL_KEY: &str = "PAEAN_USER_EMAIL";
pub const PAEAN_USER_ID_KEY: &str = "PAEAN_USER_ID";
const DEFAULT_WEB_URL: &str = "https://one.paean.ai";

#[derive(Debug, Deserialize)]
struct CallbackParams {
    token: Option<String>,
    error: Option<String>,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    email: Option<String>,
}

#[derive(Clone)]
struct AppState {
    result_sender: Arc<Mutex<Option<oneshot::Sender<CallbackParams>>>>,
}

const SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>Login Successful</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
         text-align: center; padding: 50px; background: #0a0a0a; color: #fff; }
  .icon { font-size: 64px; margin-bottom: 16px; }
  h1 { color: #22c55e; margin-bottom: 8px; }
  p { color: #a3a3a3; }
</style></head>
<body>
  <div class="icon">&#x2705;</div>
  <h1>Login Successful!</h1>
  <p>You can close this window and return to the terminal.</p>
</body></html>"#;

const FAILURE_HTML: &str = r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>Login Failed</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
         text-align: center; padding: 50px; background: #0a0a0a; color: #fff; }
  .icon { font-size: 64px; margin-bottom: 16px; }
  h1 { color: #ef4444; margin-bottom: 8px; }
  p { color: #a3a3a3; }
</style></head>
<body>
  <div class="icon">&#x274C;</div>
  <h1>Login Failed</h1>
  <p>Please return to the terminal and try again.</p>
</body></html>"#;

fn get_web_url() -> String {
    Config::global()
        .get_param::<String>("PAEAN_WEB_URL")
        .unwrap_or_else(|_| DEFAULT_WEB_URL.to_string())
}

pub fn is_paean_authenticated() -> bool {
    let config = Config::global();
    config.get_secret::<String>(PAEAN_JWT_TOKEN_KEY).is_ok()
}

pub fn get_stored_email() -> Option<String> {
    Config::global()
        .get_secret::<String>(PAEAN_USER_EMAIL_KEY)
        .ok()
}

pub fn get_stored_token() -> Option<String> {
    Config::global()
        .get_secret::<String>(PAEAN_JWT_TOKEN_KEY)
        .ok()
}

pub async fn handle_login() -> Result<()> {
    if is_paean_authenticated() {
        let email = get_stored_email().unwrap_or_else(|| "unknown".to_string());
        println!(
            "\n  {} Already logged in as {}",
            style("\u{2713}").green(),
            style(&email).cyan()
        );
        println!("  {}", style("Use \"a8e logout\" to sign out.").dim());
        return Ok(());
    }

    println!(
        "\n  {} {}",
        style("\u{221e}").magenta().bold(),
        style("Paean AI Login").bold()
    );

    let (result_tx, result_rx) = oneshot::channel::<CallbackParams>();
    let app_state = AppState {
        result_sender: Arc::new(Mutex::new(Some(result_tx))),
    };

    let handler = |Query(params): Query<CallbackParams>, State(state): State<AppState>| async move {
        let has_token = params.token.is_some();
        if let Some(sender) = state.result_sender.lock().await.take() {
            let _ = sender.send(params);
        }
        if has_token {
            Html(SUCCESS_HTML.to_string())
        } else {
            Html(FAILURE_HTML.to_string())
        }
    };

    let app = Router::new()
        .route("/callback", get(handler))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let port = listener.local_addr()?.port();

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("Login callback server error: {}", e);
        }
    });

    let callback_url = format!("http://localhost:{}/callback", port);
    let web_url = get_web_url();
    let login_url = format!(
        "{}/auth/cli?callback={}",
        web_url,
        urlencoding::encode(&callback_url)
    );

    println!(
        "\n  {} Opening browser for login...",
        style("\u{1f310}").bold()
    );
    println!(
        "  {}",
        style(format!("If browser doesn't open, visit: {}", login_url)).dim()
    );

    if webbrowser::open(&login_url).is_err() {
        println!(
            "\n  {} Could not open browser. Please visit:",
            style("\u{26a0}").yellow()
        );
        println!("  {}", style(&login_url).cyan().underlined());
    }

    println!("\n  {} Waiting for login...", style("\u{23f3}").bold());

    let timeout = tokio::time::timeout(std::time::Duration::from_secs(300), result_rx).await;

    match timeout {
        Ok(Ok(params)) => {
            if let Some(token) = params.token {
                let config = Config::global();
                config.set_secret(PAEAN_JWT_TOKEN_KEY, &token)?;

                if let Some(ref email) = params.email {
                    config.set_secret(PAEAN_USER_EMAIL_KEY, email)?;
                }
                if let Some(ref user_id) = params.user_id {
                    config.set_secret(PAEAN_USER_ID_KEY, user_id)?;
                }

                println!("\n  {} Login successful!", style("\u{2713}").green().bold());
                if let Some(email) = params.email {
                    println!("  {} {}", style("Email:").dim(), style(&email).cyan());
                }
                println!(
                    "\n  {}",
                    style("You can now use Paean AI models without an API key.").dim()
                );
            } else {
                let error_msg = params.error.unwrap_or_else(|| "Unknown error".to_string());
                println!(
                    "\n  {} Login failed: {}",
                    style("\u{2718}").red().bold(),
                    style(&error_msg).red()
                );
            }
        }
        Ok(Err(_)) => {
            println!("\n  {} Login cancelled.", style("\u{2718}").red());
        }
        Err(_) => {
            println!(
                "\n  {} Login timed out. Please try again.",
                style("\u{2718}").red()
            );
        }
    }

    Ok(())
}

pub async fn handle_login_check() -> Result<()> {
    println!(
        "\n  {} {}",
        style("\u{221e}").magenta().bold(),
        style("Authentication Status").bold()
    );

    if !is_paean_authenticated() {
        println!("\n  {} Not logged in", style("\u{26a0}").yellow());
        println!(
            "  {}",
            style("Use \"a8e login\" to authenticate with Paean AI.").dim()
        );
        return Ok(());
    }

    let email = get_stored_email().unwrap_or_else(|| "unknown".to_string());
    println!("\n  {} Authenticated", style("\u{2713}").green());
    println!("  {} {}", style("Email:").dim(), style(&email).cyan());

    Ok(())
}

pub async fn handle_logout() -> Result<()> {
    let config = Config::global();

    if !is_paean_authenticated() {
        println!("\n  {} Not logged in.", style("\u{26a0}").yellow());
        return Ok(());
    }

    let _ = config.delete_secret(PAEAN_JWT_TOKEN_KEY);
    let _ = config.delete_secret(PAEAN_USER_EMAIL_KEY);
    let _ = config.delete_secret(PAEAN_USER_ID_KEY);

    println!("\n  {} Logged out successfully.", style("\u{2713}").green());

    Ok(())
}
