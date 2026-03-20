use std::io::Write;
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
    pub timeout_secs: Option<u64>,
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
        if let Some(preset_name) = &hook.preset
            && !crate::hooks::is_known_preset(preset_name)
        {
            bail!(
                "hook '{}': unknown preset '{}'. Available: {}",
                hook.name,
                preset_name,
                crate::hooks::preset_names().join(", ")
            );
        }
    }
    Ok(())
}

/// Removes all `[[hooks.pre_push]]` entries that use a preset (not custom commands)
/// from `.lazyjj.toml`. Preserves other content. Deletes the file if empty afterward.
pub fn remove_preset_hooks(repo_root: &Path) -> Result<()> {
    let config_path = repo_root.join(".lazyjj.toml");
    if !config_path.is_file() {
        return Ok(());
    }
    let contents = std::fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;

    // Parse, filter out preset hooks, reserialize
    let mut doc: toml::Table =
        toml::from_str(&contents).with_context(|| "failed to parse .lazyjj.toml")?;

    if let Some(toml::Value::Table(hooks)) = doc.get_mut("hooks") {
        if let Some(toml::Value::Array(pre_push)) = hooks.get_mut("pre_push") {
            pre_push.retain(|entry| {
                // Keep entries that use `command` (custom), remove those with `preset`
                if let toml::Value::Table(t) = entry {
                    t.contains_key("command")
                } else {
                    true
                }
            });
            if pre_push.is_empty() {
                hooks.remove("pre_push");
            }
        }
        if hooks.is_empty() {
            doc.remove("hooks");
        }
    }

    let new_contents =
        toml::to_string_pretty(&doc).with_context(|| "failed to serialize config")?;
    if new_contents.trim().is_empty() {
        std::fs::remove_file(&config_path).ok();
    } else {
        std::fs::write(&config_path, new_contents)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }

    Ok(())
}

pub fn append_preset_hooks(repo_root: &Path, preset_names: &[&str]) -> Result<()> {
    if preset_names.is_empty() {
        return Ok(());
    }

    let config_path = repo_root.join(".lazyjj.toml");

    // Determine whether we need a leading newline to separate from existing content.
    let needs_leading_newline = if config_path.is_file() {
        let existing = std::fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        !existing.is_empty() && !existing.ends_with('\n')
    } else {
        false
    };

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config_path)
        .with_context(|| format!("failed to open {} for writing", config_path.display()))?;

    if needs_leading_newline {
        writeln!(file)?;
    }

    for name in preset_names {
        writeln!(file)?;
        writeln!(file, "[[hooks.pre_push]]")?;
        writeln!(file, r#"name = "{name}""#)?;
        writeln!(file, r#"preset = "{name}""#)?;
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
name = "check"
preset = "treefmt"

[[hooks.pre_push]]
name = "lint"
command = "cargo clippy -- -D warnings"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.hooks.pre_push.len(), 2);
        assert_eq!(config.hooks.pre_push[0].name, "check");
        assert_eq!(config.hooks.pre_push[0].preset.as_deref(), Some("treefmt"));
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
            timeout_secs: None,
        };
        assert!(hook.validate().is_err());
    }

    #[test]
    fn reject_hook_with_neither_preset_nor_command() {
        let hook = HookEntry {
            name: "empty".to_owned(),
            preset: None,
            command: None,
            timeout_secs: None,
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

    #[test]
    fn reject_unknown_preset_name() {
        let toml = r#"
[[hooks.pre_push]]
name = "check"
preset = "nonexistent"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let err = validate(&config).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown preset"),
            "expected 'unknown preset' in error, got: {msg}"
        );
        assert!(
            msg.contains("treefmt"),
            "expected available preset 'trunk' listed in error, got: {msg}"
        );
    }

    #[test]
    fn accept_known_preset_name() {
        let toml = r#"
[[hooks.pre_push]]
name = "check"
preset = "treefmt"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(validate(&config).is_ok());
    }

    fn make_temp_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lazyjj_test_{suffix}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn append_preset_hooks_creates_file() {
        let root = make_temp_dir("append_creates");
        let config_path = root.join(".lazyjj.toml");
        let _ = std::fs::remove_file(&config_path);

        append_preset_hooks(&root, &["treefmt"]).unwrap();
        let config = load(&root).unwrap();
        assert_eq!(config.hooks.pre_push.len(), 1);
        assert_eq!(config.hooks.pre_push[0].preset.as_deref(), Some("treefmt"));

        let _ = std::fs::remove_file(&config_path);
    }

    #[test]
    fn append_preset_hooks_appends_to_existing() {
        let root = make_temp_dir("append_existing");
        let config_path = root.join(".lazyjj.toml");
        std::fs::write(&config_path, "# existing\n").unwrap();

        append_preset_hooks(&root, &["biome"]).unwrap();
        let contents = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            contents.starts_with("# existing"),
            "existing content should be preserved at start, got: {contents}"
        );
        assert!(
            contents.contains("biome"),
            "appended preset should appear in file, got: {contents}"
        );

        let _ = std::fs::remove_file(&config_path);
    }

    #[test]
    fn append_preset_hooks_noop_for_empty_list() {
        let root = make_temp_dir("append_noop");
        let config_path = root.join(".lazyjj.toml");
        let _ = std::fs::remove_file(&config_path);

        append_preset_hooks(&root, &[]).unwrap();
        assert!(
            !config_path.exists(),
            "no file should be created for empty preset list"
        );
    }
}
