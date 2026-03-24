use super::base::Config;
use crate::agents::extension::PLATFORM_EXTENSIONS;
use crate::agents::ExtensionConfig;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use tracing::warn;
use utoipa::ToSchema;

pub const DEFAULT_EXTENSION: &str = "developer";
pub const DEFAULT_EXTENSION_TIMEOUT: u64 = 300;
pub const DEFAULT_EXTENSION_DESCRIPTION: &str = "";
pub const DEFAULT_DISPLAY_NAME: &str = "Developer";
const EXTENSIONS_CONFIG_KEY: &str = "extensions";

#[derive(Debug, Deserialize, Serialize, Clone, ToSchema)]
pub struct ExtensionEntry {
    pub enabled: bool,
    #[serde(flatten)]
    pub config: ExtensionConfig,
}

pub fn name_to_key(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for c in name.chars() {
        result.push(match c {
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
            c if c.is_whitespace() => continue,
            _ => '_',
        });
    }
    result.to_lowercase()
}

pub(crate) fn is_extension_available(config: &ExtensionConfig) -> bool {
    match config {
        ExtensionConfig::Platform { name, .. } => {
            PLATFORM_EXTENSIONS.contains_key(name_to_key(name).as_str())
        }
        _ => true,
    }
}

fn get_extensions_map_with_config(config: &Config) -> IndexMap<String, ExtensionEntry> {
    let raw: Mapping = config
        .get_param(EXTENSIONS_CONFIG_KEY)
        .unwrap_or_else(|err| {
            warn!(
                "Failed to load {}: {err}. Falling back to empty object.",
                EXTENSIONS_CONFIG_KEY
            );
            Default::default()
        });

    let mut extensions_map = IndexMap::with_capacity(raw.len());
    for (k, v) in raw {
        match (k, serde_yaml::from_value::<ExtensionEntry>(v)) {
            (serde_yaml::Value::String(key), Ok(entry)) => {
                if !is_extension_available(&entry.config) {
                    continue;
                }
                extensions_map.insert(key, entry);
            }
            (k, v) => {
                warn!(
                    key = ?k,
                    value = ?v,
                    "Skipping malformed extension config entry"
                );
            }
        }
    }

    extensions_map
}

fn get_extensions_map() -> IndexMap<String, ExtensionEntry> {
    get_extensions_map_with_config(Config::global())
}

fn save_extensions_map(extensions: IndexMap<String, ExtensionEntry>) {
    let config = Config::global();
    if let Err(e) = config.set_param(EXTENSIONS_CONFIG_KEY, &extensions) {
        // TODO(jack) why is this just a debug statement?
        tracing::debug!("Failed to save extensions config: {}", e);
    }
}

pub fn get_extension_by_name(name: &str) -> Option<ExtensionConfig> {
    let extensions = get_extensions_map();
    extensions
        .values()
        .find(|entry| entry.config.name() == name)
        .map(|entry| entry.config.clone())
}

pub fn set_extension(entry: ExtensionEntry) {
    let mut extensions = get_extensions_map();
    let key = entry.config.key();
    extensions.insert(key, entry);
    save_extensions_map(extensions);
}

pub fn remove_extension(key: &str) {
    let mut extensions = get_extensions_map();
    extensions.shift_remove(key);
    save_extensions_map(extensions);
}

pub fn set_extension_enabled(key: &str, enabled: bool) {
    let mut extensions = get_extensions_map();
    if let Some(entry) = extensions.get_mut(key) {
        entry.enabled = enabled;
        save_extensions_map(extensions);
    }
}

pub fn get_all_extensions() -> Vec<ExtensionEntry> {
    let extensions = get_extensions_map();
    extensions.into_values().collect()
}

pub fn get_all_extension_names() -> Vec<String> {
    let extensions = get_extensions_map();
    extensions.keys().cloned().collect()
}

pub fn is_extension_enabled(key: &str) -> bool {
    let extensions = get_extensions_map();
    extensions.get(key).map(|e| e.enabled).unwrap_or(false)
}

pub fn get_enabled_extensions() -> Vec<ExtensionConfig> {
    get_all_extensions()
        .into_iter()
        .filter(|ext| ext.enabled)
        .map(|ext| ext.config)
        .collect()
}

pub fn get_enabled_extensions_with_config(config: &Config) -> Vec<ExtensionConfig> {
    get_extensions_map_with_config(config)
        .into_values()
        .filter(|ext| ext.enabled)
        .map(|ext| ext.config)
        .collect()
}

pub fn get_warnings() -> Vec<String> {
    let raw: Mapping = Config::global()
        .get_param(EXTENSIONS_CONFIG_KEY)
        .unwrap_or_default();

    let mut warnings = Vec::new();
    for (k, v) in raw {
        if let (serde_yaml::Value::String(key), Ok(entry)) =
            (k, serde_yaml::from_value::<ExtensionEntry>(v))
        {
            if matches!(entry.config, ExtensionConfig::Sse { .. }) {
                warnings.push(format!(
                    "'{}': SSE is unsupported, migrate to streamable_http",
                    key
                ));
            }
        }
    }
    warnings
}

/// Builtin extensions that are always injected into every session.
const AUTO_BUILTINS: &[(&str, &str)] = &[("loop", "Session-scoped loop tasks")];

fn inject_auto_builtins(mut extensions: Vec<ExtensionConfig>) -> Vec<ExtensionConfig> {
    for &(name, description) in AUTO_BUILTINS {
        let already_present = extensions.iter().any(|e| {
            if let ExtensionConfig::Builtin { name: ref n, .. } = e {
                n == name
            } else {
                false
            }
        });
        if !already_present {
            extensions.push(ExtensionConfig::Builtin {
                name: name.to_string(),
                display_name: None,
                description: description.to_string(),
                timeout: Some(DEFAULT_EXTENSION_TIMEOUT),
                bundled: Some(true),
                available_tools: vec![],
            });
        }
    }
    extensions
}

pub fn resolve_extensions_for_new_session(
    recipe_extensions: Option<&[ExtensionConfig]>,
    override_extensions: Option<Vec<ExtensionConfig>>,
) -> Vec<ExtensionConfig> {
    let extensions = if let Some(exts) = recipe_extensions {
        exts.to_vec()
    } else if let Some(exts) = override_extensions {
        exts
    } else {
        get_enabled_extensions()
    };

    let extensions = inject_auto_builtins(extensions);

    extensions
        .into_iter()
        .filter(is_extension_available)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_extension_available_filters_unknown_platform() {
        let unknown_platform = ExtensionConfig::Platform {
            name: "definitely_not_real_platform_extension".to_string(),
            description: "unknown".to_string(),
            display_name: None,
            bundled: None,
            available_tools: Vec::new(),
        };

        let builtin = ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "".to_string(),
            display_name: Some("Developer".to_string()),
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        };

        assert!(!is_extension_available(&unknown_platform));
        assert!(is_extension_available(&builtin));
    }

    #[test]
    fn test_inject_auto_builtins_adds_loop() {
        let extensions = vec![ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "".to_string(),
            display_name: Some("Developer".to_string()),
            timeout: None,
            bundled: None,
            available_tools: vec![],
        }];

        let result = inject_auto_builtins(extensions);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|e| e.name() == "loop"));
    }

    #[test]
    fn test_inject_auto_builtins_does_not_duplicate() {
        let extensions = vec![
            ExtensionConfig::Builtin {
                name: "developer".to_string(),
                description: "".to_string(),
                display_name: Some("Developer".to_string()),
                timeout: None,
                bundled: None,
                available_tools: vec![],
            },
            ExtensionConfig::Builtin {
                name: "loop".to_string(),
                description: "custom".to_string(),
                display_name: None,
                timeout: Some(60),
                bundled: Some(true),
                available_tools: vec![],
            },
        ];

        let result = inject_auto_builtins(extensions);
        assert_eq!(result.len(), 2);
        let loop_ext = result.iter().find(|e| e.name() == "loop").unwrap();
        if let ExtensionConfig::Builtin { description, .. } = loop_ext {
            assert_eq!(description, "custom");
        } else {
            panic!("loop should be a Builtin");
        }
    }

    #[test]
    fn test_inject_auto_builtins_on_empty() {
        let result = inject_auto_builtins(vec![]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name(), "loop");
    }
}
