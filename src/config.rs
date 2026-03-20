use std::fmt::Write as FmtWrite;
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

/// Appends command-based hook entries to `.lazyjj.toml`.
///
/// `tools` is a slice of `(name, command)` pairs.  Each pair becomes a
/// `[[hooks.pre_push]]` entry with a `command` field (not a `preset` field).
pub fn write_tool_hooks(repo_root: &Path, tools: &[(&str, &str)]) -> Result<()> {
    if tools.is_empty() {
        return Ok(());
    }

    let config_path = repo_root.join(".lazyjj.toml");

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

    for (name, command) in tools {
        writeln!(file)?;
        writeln!(file, "[[hooks.pre_push]]")?;
        writeln!(file, r#"name = "{name}""#)?;
        writeln!(file, r#"command = "{command}""#)?;
    }

    Ok(())
}

/// Kept for backward compatibility with existing `.lazyjj.toml` files that use the `preset` field.
/// New code should prefer [`write_tool_hooks`] which writes `command`-based entries.
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

/// Read the Rust edition from Cargo.toml in the given directory.
/// Returns None if no Cargo.toml exists or the edition field is absent.
fn detect_rust_edition(repo_root: &Path) -> Option<String> {
    let cargo_path = repo_root.join("Cargo.toml");
    let contents = std::fs::read_to_string(cargo_path).ok()?;
    let table: toml::Table = toml::from_str(&contents).ok()?;
    let package = table.get("package")?.as_table()?;
    let edition = package.get("edition")?.as_str()?;
    Some(edition.to_owned())
}

const TREEFMT_GLOBAL_EXCLUDES: &[&str] = &[
    "node_modules/**",
    "target/**",
    ".git/**",
    "dist/**",
    "build/**",
    ".venv/**",
    "__pycache__/**",
];

/// Generates a `treefmt.toml` file in the repo root for the given formatters.
///
/// Returns `Ok(true)` if the file was created, `Ok(false)` if it already existed or
/// no formatters were provided (in which case the file is not touched).
pub fn generate_treefmt_toml(
    repo_root: &Path,
    formatters: &[&crate::hooks::ToolDef],
) -> Result<bool> {
    if formatters.is_empty() {
        return Ok(false);
    }
    let path = repo_root.join("treefmt.toml");
    if path.is_file() {
        return Ok(false);
    }

    let mut content = String::new();
    writeln!(content, "# Generated by lazyjj — edit freely").unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[global]").unwrap();
    writeln!(content, "excludes = [").unwrap();
    for e in TREEFMT_GLOBAL_EXCLUDES {
        writeln!(content, "  \"{e}\",").unwrap();
    }
    writeln!(content, "]").unwrap();

    // Detect Rust edition from Cargo.toml for rustfmt
    let rust_edition = detect_rust_edition(repo_root);

    for tool in formatters {
        let Some(entry) = &tool.treefmt_entry else {
            continue;
        };
        writeln!(content).unwrap();
        writeln!(content, "[formatter.{}]", entry.name).unwrap();
        writeln!(content, r#"command = "{}""#, entry.command).unwrap();

        // For rustfmt, inject --edition from Cargo.toml
        let mut options: Vec<&str> = entry.options.to_vec();
        if entry.name == "rustfmt"
            && let Some(edition) = &rust_edition
        {
            options.push("--edition");
            options.push(edition);
        }

        if !options.is_empty() {
            let opts: Vec<String> = options.iter().map(|o| format!("\"{o}\"")).collect();
            writeln!(content, "options = [{}]", opts.join(", ")).unwrap();
        }

        // includes array
        let incs: Vec<String> = entry.includes.iter().map(|i| format!("\"{i}\"")).collect();
        writeln!(content, "includes = [{}]", incs.join(", ")).unwrap();
    }

    std::fs::write(&path, &content)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(true)
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
            "expected available preset 'treefmt' listed in error, got: {msg}"
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

    #[test]
    fn write_tool_hooks_creates_command_entries() {
        let root = make_temp_dir("write_tools_creates");
        let config_path = root.join(".lazyjj.toml");
        let _ = std::fs::remove_file(&config_path);

        write_tool_hooks(&root, &[("rustfmt", "cargo fmt -- --check")]).unwrap();
        let config = load(&root).unwrap();
        assert_eq!(config.hooks.pre_push.len(), 1);
        assert_eq!(config.hooks.pre_push[0].name, "rustfmt");
        assert_eq!(
            config.hooks.pre_push[0].command.as_deref(),
            Some("cargo fmt -- --check")
        );
        assert!(config.hooks.pre_push[0].preset.is_none());

        let _ = std::fs::remove_file(&config_path);
    }

    #[test]
    fn write_tool_hooks_appends_multiple_tools() {
        let root = make_temp_dir("write_tools_multiple");
        let config_path = root.join(".lazyjj.toml");
        let _ = std::fs::remove_file(&config_path);

        write_tool_hooks(
            &root,
            &[
                ("rustfmt", "cargo fmt -- --check"),
                ("clippy", "cargo clippy -- -D warnings"),
            ],
        )
        .unwrap();
        let config = load(&root).unwrap();
        assert_eq!(config.hooks.pre_push.len(), 2);
        assert_eq!(config.hooks.pre_push[0].name, "rustfmt");
        assert_eq!(config.hooks.pre_push[1].name, "clippy");

        let _ = std::fs::remove_file(&config_path);
    }

    #[test]
    fn write_tool_hooks_noop_for_empty_list() {
        let root = make_temp_dir("write_tools_noop");
        let config_path = root.join(".lazyjj.toml");
        let _ = std::fs::remove_file(&config_path);

        write_tool_hooks(&root, &[]).unwrap();
        assert!(
            !config_path.exists(),
            "no file should be created for empty tool list"
        );
    }

    #[test]
    fn generate_treefmt_toml_creates_file() {
        use crate::hooks::TOOL_CATALOG;

        let root = make_temp_dir("treefmt_creates");
        let treefmt_path = root.join("treefmt.toml");
        let _ = std::fs::remove_file(&treefmt_path);

        // Pick any formatter from the catalog (must have treefmt_entry)
        let formatters: Vec<&crate::hooks::ToolDef> = TOOL_CATALOG
            .iter()
            .filter(|t| t.treefmt_entry.is_some())
            .collect();
        assert!(
            !formatters.is_empty(),
            "need at least one formatter in TOOL_CATALOG to run this test"
        );

        let result = generate_treefmt_toml(&root, &formatters).unwrap();
        assert!(
            result,
            "generate_treefmt_toml should return Ok(true) when creating a new file"
        );
        assert!(
            treefmt_path.exists(),
            "treefmt.toml should have been created"
        );

        let contents = std::fs::read_to_string(&treefmt_path).unwrap();
        assert!(
            contents.contains("[global]"),
            "treefmt.toml must contain [global] section, got:\n{contents}"
        );
        assert!(
            contents.contains("excludes"),
            "treefmt.toml must contain excludes key, got:\n{contents}"
        );
        // The first formatter's treefmt_entry should produce a [formatter.<name>] section
        let first = formatters[0];
        let entry = first.treefmt_entry.as_ref().unwrap();
        assert!(
            contents.contains(&format!("[formatter.{}]", entry.name)),
            "treefmt.toml must contain [formatter.{}], got:\n{contents}",
            entry.name
        );
        assert!(
            contents.contains("command"),
            "treefmt.toml must contain command key, got:\n{contents}"
        );
        assert!(
            contents.contains("includes"),
            "treefmt.toml must contain includes key, got:\n{contents}"
        );

        let _ = std::fs::remove_file(&treefmt_path);
    }

    #[test]
    fn generate_treefmt_toml_skips_existing() {
        use crate::hooks::TOOL_CATALOG;

        let root = make_temp_dir("treefmt_skips");
        let treefmt_path = root.join("treefmt.toml");
        std::fs::write(&treefmt_path, "# custom\n").unwrap();

        let formatters: Vec<&crate::hooks::ToolDef> = TOOL_CATALOG
            .iter()
            .filter(|t| t.treefmt_entry.is_some())
            .collect();

        let result = generate_treefmt_toml(&root, &formatters).unwrap();
        assert!(
            !result,
            "generate_treefmt_toml should return Ok(false) when file already exists"
        );

        let contents = std::fs::read_to_string(&treefmt_path).unwrap();
        assert_eq!(
            contents, "# custom\n",
            "pre-existing treefmt.toml must not be modified"
        );

        let _ = std::fs::remove_file(&treefmt_path);
    }

    #[test]
    fn generate_treefmt_toml_noop_for_empty() {
        let root = make_temp_dir("treefmt_noop");
        let treefmt_path = root.join("treefmt.toml");
        let _ = std::fs::remove_file(&treefmt_path);

        generate_treefmt_toml(&root, &[]).unwrap();
        assert!(
            !treefmt_path.exists(),
            "treefmt.toml should not be created when no formatters are provided"
        );
    }
}
