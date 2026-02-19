use crate::agents::extension::PLATFORM_EXTENSIONS;
use crate::agents::ExtensionConfig;
use crate::config::extensions::{
    ExtensionEntry, DEFAULT_DISPLAY_NAME, DEFAULT_EXTENSION, DEFAULT_EXTENSION_TIMEOUT,
};
use serde_yaml::Mapping;

const EXTENSIONS_CONFIG_KEY: &str = "extensions";

struct BuiltinExtensionDef {
    name: &'static str,
    display_name: &'static str,
    default_enabled: bool,
    timeout: u64,
}

const BUILTIN_EXTENSIONS: &[BuiltinExtensionDef] = &[BuiltinExtensionDef {
    name: DEFAULT_EXTENSION,
    display_name: DEFAULT_DISPLAY_NAME,
    default_enabled: true,
    timeout: DEFAULT_EXTENSION_TIMEOUT,
}];

pub fn run_migrations(config: &mut Mapping) -> bool {
    let mut changed = false;
    changed |= migrate_platform_extensions(config);
    changed |= migrate_builtin_extensions(config);
    changed
}

fn migrate_platform_extensions(config: &mut Mapping) -> bool {
    let extensions_key = serde_yaml::Value::String(EXTENSIONS_CONFIG_KEY.to_string());

    let extensions_value = config
        .get(&extensions_key)
        .cloned()
        .unwrap_or(serde_yaml::Value::Mapping(Mapping::new()));

    let mut extensions_map: Mapping = match extensions_value {
        serde_yaml::Value::Mapping(m) => m,
        _ => Mapping::new(),
    };

    let mut needs_save = false;

    for (name, def) in PLATFORM_EXTENSIONS.iter() {
        let ext_key = serde_yaml::Value::String(name.to_string());
        let existing = extensions_map.get(&ext_key);

        let needs_migration = match existing {
            None => true,
            Some(value) => match serde_yaml::from_value::<ExtensionEntry>(value.clone()) {
                Ok(entry) => {
                    if let ExtensionConfig::Platform {
                        description,
                        display_name,
                        ..
                    } = &entry.config
                    {
                        description != def.description
                            || display_name.as_deref() != Some(def.display_name)
                    } else {
                        true
                    }
                }
                Err(_) => true,
            },
        };

        if needs_migration {
            let enabled = existing
                .and_then(|v| serde_yaml::from_value::<ExtensionEntry>(v.clone()).ok())
                .map(|e| e.enabled)
                .unwrap_or(def.default_enabled);

            let new_entry = ExtensionEntry {
                config: ExtensionConfig::Platform {
                    name: def.name.to_string(),
                    description: def.description.to_string(),
                    display_name: Some(def.display_name.to_string()),
                    bundled: Some(true),
                    available_tools: Vec::new(),
                },
                enabled,
            };

            if let Ok(value) = serde_yaml::to_value(&new_entry) {
                extensions_map.insert(ext_key, value);
                needs_save = true;
            }
        }
    }

    if needs_save {
        config.insert(extensions_key, serde_yaml::Value::Mapping(extensions_map));
    }

    needs_save
}

fn migrate_builtin_extensions(config: &mut Mapping) -> bool {
    let extensions_key = serde_yaml::Value::String(EXTENSIONS_CONFIG_KEY.to_string());

    let extensions_value = config
        .get(&extensions_key)
        .cloned()
        .unwrap_or(serde_yaml::Value::Mapping(Mapping::new()));

    let mut extensions_map: Mapping = match extensions_value {
        serde_yaml::Value::Mapping(m) => m,
        _ => Mapping::new(),
    };

    let mut needs_save = false;

    for def in BUILTIN_EXTENSIONS {
        let ext_key = serde_yaml::Value::String(def.name.to_string());

        if extensions_map.contains_key(&ext_key) {
            continue;
        }

        let new_entry = ExtensionEntry {
            config: ExtensionConfig::Builtin {
                name: def.name.to_string(),
                display_name: Some(def.display_name.to_string()),
                description: String::new(),
                timeout: Some(def.timeout),
                bundled: Some(true),
                available_tools: Vec::new(),
            },
            enabled: def.default_enabled,
        };

        if let Ok(value) = serde_yaml::to_value(&new_entry) {
            extensions_map.insert(ext_key, value);
            needs_save = true;
        }
    }

    if needs_save {
        config.insert(extensions_key, serde_yaml::Value::Mapping(extensions_map));
    }

    needs_save
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_platform_extensions_empty_config() {
        let mut config = Mapping::new();
        let changed = run_migrations(&mut config);

        assert!(changed);
        let extensions_key = serde_yaml::Value::String(EXTENSIONS_CONFIG_KEY.to_string());
        assert!(config.contains_key(&extensions_key));
    }

    #[test]
    fn test_migrate_platform_extensions_preserves_enabled_state() {
        let mut config = Mapping::new();
        let mut extensions = Mapping::new();
        let todo_entry = ExtensionEntry {
            config: ExtensionConfig::Platform {
                name: "todo".to_string(),
                description: "old description".to_string(),
                display_name: Some("Old Name".to_string()),
                bundled: Some(true),
                available_tools: Vec::new(),
            },
            enabled: false,
        };
        extensions.insert(
            serde_yaml::Value::String("todo".to_string()),
            serde_yaml::to_value(&todo_entry).unwrap(),
        );
        config.insert(
            serde_yaml::Value::String(EXTENSIONS_CONFIG_KEY.to_string()),
            serde_yaml::Value::Mapping(extensions),
        );

        let changed = run_migrations(&mut config);
        assert!(changed);

        let extensions_key = serde_yaml::Value::String(EXTENSIONS_CONFIG_KEY.to_string());
        let extensions = config.get(&extensions_key).unwrap().as_mapping().unwrap();
        let todo_key = serde_yaml::Value::String("todo".to_string());
        let todo_value = extensions.get(&todo_key).unwrap();
        let todo_entry: ExtensionEntry = serde_yaml::from_value(todo_value.clone()).unwrap();

        assert!(!todo_entry.enabled);
    }

    #[test]
    fn test_migrate_platform_extensions_idempotent() {
        let mut config = Mapping::new();
        run_migrations(&mut config);

        let changed = run_migrations(&mut config);
        assert!(!changed);
    }

    #[test]
    fn test_migrate_adds_developer_builtin_to_empty_config() {
        let mut config = Mapping::new();
        let changed = run_migrations(&mut config);

        assert!(changed);
        let extensions_key = serde_yaml::Value::String(EXTENSIONS_CONFIG_KEY.to_string());
        let extensions = config.get(&extensions_key).unwrap().as_mapping().unwrap();
        let dev_key = serde_yaml::Value::String("developer".to_string());
        let dev_value = extensions.get(&dev_key).unwrap();
        let dev_entry: ExtensionEntry = serde_yaml::from_value(dev_value.clone()).unwrap();

        assert!(dev_entry.enabled);
        assert!(matches!(dev_entry.config, ExtensionConfig::Builtin { ref name, .. } if name == "developer"));
    }

    #[test]
    fn test_migrate_preserves_existing_developer_config() {
        let mut config = Mapping::new();
        let mut extensions = Mapping::new();
        let dev_entry = ExtensionEntry {
            config: ExtensionConfig::Builtin {
                name: "developer".to_string(),
                display_name: Some("Developer".to_string()),
                description: "custom".to_string(),
                timeout: Some(600),
                bundled: Some(true),
                available_tools: Vec::new(),
            },
            enabled: false,
        };
        extensions.insert(
            serde_yaml::Value::String("developer".to_string()),
            serde_yaml::to_value(&dev_entry).unwrap(),
        );
        config.insert(
            serde_yaml::Value::String(EXTENSIONS_CONFIG_KEY.to_string()),
            serde_yaml::Value::Mapping(extensions),
        );

        run_migrations(&mut config);

        let extensions_key = serde_yaml::Value::String(EXTENSIONS_CONFIG_KEY.to_string());
        let extensions = config.get(&extensions_key).unwrap().as_mapping().unwrap();
        let dev_key = serde_yaml::Value::String("developer".to_string());
        let dev_value = extensions.get(&dev_key).unwrap();
        let dev_entry: ExtensionEntry = serde_yaml::from_value(dev_value.clone()).unwrap();

        assert!(!dev_entry.enabled);
        if let ExtensionConfig::Builtin { timeout, .. } = &dev_entry.config {
            assert_eq!(*timeout, Some(600));
        }
    }
}
