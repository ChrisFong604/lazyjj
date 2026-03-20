use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hooks: HooksConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct HooksConfig {
    pub pre_push: Vec<HookEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HookEntry {
    pub name: String,
    pub preset: Option<String>,
    pub command: Option<String>,
}

impl HookEntry {
    fn validate(&self) -> Result<()> {
        match (&self.preset, &self.command) {
            (Some(_), Some(_)) => {
                bail!(
                    "hook '{}': specify either `preset` or `command`, not both",
                    self.name
                );
            }
            (None, None) => {
                bail!(
                    "hook '{}': must specify either `preset` or `command`",
                    self.name
                );
            }
            _ => Ok(()),
        }
    }
}

pub fn load(repo_root: &Path) -> Result<Config> {
    let path = resolve_config_path(repo_root);
    let Some(path) = path else {
        return Ok(Config::default());
    };
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config from {}", path.display()))?;
    let config: Config = toml::from_str(&contents)
        .with_context(|| format!("failed to parse config from {}", path.display()))?;
    validate(&config)?;
    Ok(config)
}

fn resolve_config_path(repo_root: &Path) -> Option<PathBuf> {
    let local = repo_root.join(".lazyjj.toml");
    if local.is_file() {
        return Some(local);
    }
    if let Some(config_dir) = dirs_config_dir() {
        let global = config_dir.join("lazyjj").join("config.toml");
        if global.is_file() {
            return Some(global);
        }
    }
    None
}

fn dirs_config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

fn validate(config: &Config) -> Result<()> {
    for hook in &config.hooks.pre_push {
        hook.validate()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_empty_hooks() {
        let config = Config::default();
        assert!(config.hooks.pre_push.is_empty());
    }

    #[test]
    fn parse_valid_config() {
        let toml = r#"
[[hooks.pre_push]]
name = "format"
preset = "biome"

[[hooks.pre_push]]
name = "lint"
command = "cargo clippy -- -D warnings"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.hooks.pre_push.len(), 2);
        assert_eq!(config.hooks.pre_push[0].name, "format");
        assert_eq!(config.hooks.pre_push[0].preset.as_deref(), Some("biome"));
        assert!(config.hooks.pre_push[0].command.is_none());
        assert_eq!(config.hooks.pre_push[1].name, "lint");
        assert!(config.hooks.pre_push[1].preset.is_none());
        assert_eq!(
            config.hooks.pre_push[1].command.as_deref(),
            Some("cargo clippy -- -D warnings")
        );
    }

    #[test]
    fn reject_hook_with_both_preset_and_command() {
        let hook = HookEntry {
            name: "bad".to_owned(),
            preset: Some("biome".to_owned()),
            command: Some("echo hi".to_owned()),
        };
        assert!(hook.validate().is_err());
    }

    #[test]
    fn reject_hook_with_neither_preset_nor_command() {
        let hook = HookEntry {
            name: "empty".to_owned(),
            preset: None,
            command: None,
        };
        assert!(hook.validate().is_err());
    }

    #[test]
    fn empty_toml_produces_default() {
        let config: Config = toml::from_str("").unwrap();
        assert!(config.hooks.pre_push.is_empty());
    }

    #[test]
    fn missing_config_file_returns_default() {
        let config = load(Path::new("/nonexistent/path")).unwrap();
        assert!(config.hooks.pre_push.is_empty());
    }
}
