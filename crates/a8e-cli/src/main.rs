use a8e::cli::cli;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = a8e::logging::setup_logging(None);

    cli().await
}
