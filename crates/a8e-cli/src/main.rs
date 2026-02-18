use anyhow::Result;
use a8e::cli::cli;

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = a8e::logging::setup_logging(None) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }

    let result = cli().await;

    if a8e_core::otel::otlp::is_otlp_initialized() {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        a8e_core::otel::otlp::shutdown_otlp();
    }

    result
}
