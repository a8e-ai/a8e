use a8e::cli::cli;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = a8e::logging::setup_logging(None);

    let result = cli().await;

    if a8e_core::otel::otlp::is_otlp_initialized() {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        a8e_core::otel::otlp::shutdown_otlp();
    }

    result
}
