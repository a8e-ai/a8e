use etcetera::AppStrategyArgs;
use once_cell::sync::Lazy;
use rmcp::{ServerHandler, ServiceExt};
use std::collections::HashMap;

pub static APP_STRATEGY: Lazy<AppStrategyArgs> = Lazy::new(|| AppStrategyArgs {
    top_level_domain: "Block".to_string(),
    author: "Block".to_string(),
    app_name: "a8e".to_string(),
});

pub mod autovisualiser;
pub mod computercontroller;
pub mod developer;
pub mod loop_scheduler;
pub mod mcp_server_runner;
mod memory;
pub mod subprocess;
pub mod tutorial;

pub use autovisualiser::AutoVisualiserRouter;
pub use computercontroller::ComputerControllerServer;
pub use developer::rmcp_developer::DeveloperServer;
pub use developer::rmcp_developer::WORKING_DIR_PLACEHOLDER;
pub use loop_scheduler::LoopServer;
pub use loop_scheduler::{set_agent_busy, take_loop_prompt_receiver, LoopPromptEvent};
pub use memory::MemoryServer;
pub use tutorial::TutorialServer;

// Legacy aliases for backward compatibility
pub use loop_scheduler::CronServer;
pub use loop_scheduler::{take_cron_prompt_receiver, CronPromptEvent};

/// Type definition for a function that spawns and serves a builtin extension server
pub type SpawnServerFn = fn(tokio::io::DuplexStream, tokio::io::DuplexStream);

fn spawn_and_serve<S>(
    name: &'static str,
    server: S,
    transport: (tokio::io::DuplexStream, tokio::io::DuplexStream),
) where
    S: ServerHandler + Send + 'static,
{
    tokio::spawn(async move {
        match server.serve(transport).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            Err(e) => tracing::error!(builtin = name, error = %e, "server error"),
        }
    });
}

macro_rules! builtin {
    ($name:ident, $server_ty:ty) => {{
        fn spawn(r: tokio::io::DuplexStream, w: tokio::io::DuplexStream) {
            spawn_and_serve(stringify!($name), <$server_ty>::new(), (r, w));
        }
        (stringify!($name), spawn as SpawnServerFn)
    }};
}

pub static BUILTIN_EXTENSIONS: Lazy<HashMap<&'static str, SpawnServerFn>> = Lazy::new(|| {
    HashMap::from([
        builtin!(developer, DeveloperServer),
        builtin!(autovisualiser, AutoVisualiserRouter),
        builtin!(computercontroller, ComputerControllerServer),
        builtin!(memory, MemoryServer),
        builtin!(tutorial, TutorialServer),
        builtin!(loop, LoopServer),
    ])
});
