use anyhow::Result;
use a8e_core::providers::provider_registry::ProviderConstructor;
use std::sync::Arc;
use tracing::info;

use crate::server::GooseAcpAgent;

pub struct AcpServerFactoryConfig {
    pub builtins: Vec<String>,
    pub data_dir: std::path::PathBuf,
    pub config_dir: std::path::PathBuf,
}

pub struct AcpServer {
    config: AcpServerFactoryConfig,
}

impl AcpServer {
    pub fn new(config: AcpServerFactoryConfig) -> Self {
        Self { config }
    }

    pub async fn create_agent(&self) -> Result<Arc<GooseAcpAgent>> {
        let config_path = self
            .config
            .config_dir
            .join(a8e_core::config::base::CONFIG_YAML_NAME);
        let config = a8e_core::config::Config::new(&config_path, "a8e")?;

        let goose_mode = config
            .get_a8e_mode()
            .unwrap_or(a8e_core::config::GooseMode::Auto);
        let disable_session_naming = config.get_a8e_disable_session_naming().unwrap_or(false);

        let config_dir = self.config.config_dir.clone();
        let provider_factory: ProviderConstructor = Arc::new(move |model_config, extensions| {
            let config_dir = config_dir.clone();
            Box::pin(async move {
                let config_path = config_dir.join(a8e_core::config::base::CONFIG_YAML_NAME);
                let config = a8e_core::config::Config::new(&config_path, "a8e")?;
                let provider_name = config.get_a8e_provider()?;
                a8e_core::providers::create(&provider_name, model_config, extensions).await
            })
        });

        let agent = GooseAcpAgent::new(
            provider_factory,
            self.config.builtins.clone(),
            self.config.data_dir.clone(),
            self.config.config_dir.clone(),
            goose_mode,
            disable_session_naming,
        )
        .await?;
        info!("Created new ACP agent");

        Ok(Arc::new(agent))
    }
}
