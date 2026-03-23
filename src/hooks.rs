use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::config::HookEntry;

const DEFAULT_TIMEOUT_SECS: u64 = 120;

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn spinner_char(tick: usize) -> char {
    SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
}

struct PresetDef {
    program: &'static str,
    args: &'static [&'static str],
    local_paths: &'static [&'static str],
    install_url: &'static str,
}

const PRESETS: &[(&str, PresetDef)] = &[(
    "treefmt",
    PresetDef {
        program: "treefmt",
        args: &["--fail-on-change", "--no-cache"],
        local_paths: &[],
        install_url: "https://github.com/numtide/treefmt",
    },
)];

pub fn preset_names() -> Vec<&'static str> {
    PRESETS.iter().map(|(name, _)| *name).collect()
}

pub fn is_known_preset(name: &str) -> bool {
    PRESETS.iter().any(|(n, _)| *n == name)
}

fn find_preset(name: &str) -> Option<&'static PresetDef> {
    PRESETS.iter().find(|(n, _)| *n == name).map(|(_, d)| d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    Formatter,
    Linter,
}

/// A treefmt formatter section entry.
pub struct TreefmtEntry {
    pub name: &'static str,
    pub command: &'static str,
    pub options: &'static [&'static str],
    pub includes: &'static [&'static str],
}

/// A tool that can be added to pre-push hooks via the tool picker.
pub struct ToolDef {
    pub name: &'static str,
    pub language: &'static str,
    pub hook_command: &'static str,
    /// Shell command to install the tool. Empty string means comes pre-installed.
    pub install_cmd: &'static str,
    /// Binary name to check for presence via `which`.
    pub check_binary: &'static str,
    pub category: ToolCategory,
    pub treefmt_entry: Option<TreefmtEntry>,
}

pub static TOOL_CATALOG: &[ToolDef] = &[
    // ===================== Formatters =====================
    // Rust
    ToolDef {
        name: "rustfmt",
        language: "Rust",
        hook_command: "",
        install_cmd: "rustup component add rustfmt",
        check_binary: "rustfmt",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "rustfmt",
            command: "rustfmt",
            options: &[], // --edition injected dynamically from Cargo.toml
            includes: &["*.rs"],
        }),
    },
    // JS/TS
    ToolDef {
        name: "prettier",
        language: "JS/TS",
        hook_command: "",
        install_cmd: "npm install -g prettier",
        check_binary: "prettier",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "prettier",
            command: "prettier",
            options: &["--write"],
            includes: &["*.js", "*.ts", "*.jsx", "*.tsx", "*.css", "*.json", "*.md"],
        }),
    },
    ToolDef {
        name: "biome format",
        language: "JS/TS",
        hook_command: "",
        install_cmd: "npm install -g @biomejs/biome",
        check_binary: "biome",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "biome-format",
            command: "biome",
            options: &["format", "--write"],
            includes: &["*.js", "*.ts", "*.jsx", "*.tsx", "*.json"],
        }),
    },
    // Python
    ToolDef {
        name: "ruff format",
        language: "Python",
        hook_command: "",
        install_cmd: "pip install ruff",
        check_binary: "ruff",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "ruff-format",
            command: "ruff",
            options: &["format"],
            includes: &["*.py"],
        }),
    },
    ToolDef {
        name: "black",
        language: "Python",
        hook_command: "",
        install_cmd: "pip install black",
        check_binary: "black",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "black",
            command: "black",
            options: &[],
            includes: &["*.py"],
        }),
    },
    ToolDef {
        name: "isort",
        language: "Python",
        hook_command: "",
        install_cmd: "pip install isort",
        check_binary: "isort",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "isort",
            command: "isort",
            options: &[],
            includes: &["*.py"],
        }),
    },
    // Go
    ToolDef {
        name: "gofmt",
        language: "Go",
        hook_command: "",
        install_cmd: "",
        check_binary: "gofmt",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "gofmt",
            command: "gofmt",
            options: &["-w"],
            includes: &["*.go"],
        }),
    },
    ToolDef {
        name: "gofumpt",
        language: "Go",
        hook_command: "",
        install_cmd: "go install mvdan.cc/gofumpt@latest",
        check_binary: "gofumpt",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "gofumpt",
            command: "gofumpt",
            options: &["-w"],
            includes: &["*.go"],
        }),
    },
    ToolDef {
        name: "goimports",
        language: "Go",
        hook_command: "",
        install_cmd: "go install golang.org/x/tools/cmd/goimports@latest",
        check_binary: "goimports",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "goimports",
            command: "goimports",
            options: &["-w"],
            includes: &["*.go"],
        }),
    },
    // C/C++
    ToolDef {
        name: "clang-format",
        language: "C/C++",
        hook_command: "",
        install_cmd: "brew install clang-format",
        check_binary: "clang-format",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "clang-format",
            command: "clang-format",
            options: &["-i"],
            includes: &["*.c", "*.h", "*.cpp", "*.hpp", "*.cc", "*.cxx"],
        }),
    },
    // Java
    ToolDef {
        name: "google-java-format",
        language: "Java",
        hook_command: "",
        install_cmd: "brew install google-java-format",
        check_binary: "google-java-format",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "google-java-format",
            command: "google-java-format",
            options: &["-i"],
            includes: &["*.java"],
        }),
    },
    // Kotlin
    ToolDef {
        name: "ktlint",
        language: "Kotlin",
        hook_command: "",
        install_cmd: "brew install ktlint",
        check_binary: "ktlint",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "ktlint",
            command: "ktlint",
            options: &["--format"],
            includes: &["*.kt", "*.kts"],
        }),
    },
    // Swift
    ToolDef {
        name: "swiftformat",
        language: "Swift",
        hook_command: "",
        install_cmd: "brew install swiftformat",
        check_binary: "swiftformat",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "swiftformat",
            command: "swiftformat",
            options: &[],
            includes: &["*.swift"],
        }),
    },
    // Ruby
    ToolDef {
        name: "rubocop",
        language: "Ruby",
        hook_command: "",
        install_cmd: "gem install rubocop",
        check_binary: "rubocop",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "rubocop",
            command: "rubocop",
            options: &["--autocorrect"],
            includes: &["*.rb"],
        }),
    },
    // PHP
    ToolDef {
        name: "php-cs-fixer",
        language: "PHP",
        hook_command: "",
        install_cmd: "composer global require friendsofphp/php-cs-fixer",
        check_binary: "php-cs-fixer",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "php-cs-fixer",
            command: "php-cs-fixer",
            options: &["fix"],
            includes: &["*.php"],
        }),
    },
    // Shell
    ToolDef {
        name: "shfmt",
        language: "Shell",
        hook_command: "",
        install_cmd: "brew install shfmt",
        check_binary: "shfmt",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "shfmt",
            command: "shfmt",
            options: &["-w"],
            includes: &["*.sh", "*.bash"],
        }),
    },
    // Lua
    ToolDef {
        name: "stylua",
        language: "Lua",
        hook_command: "",
        install_cmd: "cargo install stylua",
        check_binary: "stylua",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "stylua",
            command: "stylua",
            options: &[],
            includes: &["*.lua"],
        }),
    },
    // Zig
    ToolDef {
        name: "zig fmt",
        language: "Zig",
        hook_command: "",
        install_cmd: "",
        check_binary: "zig",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "zig-fmt",
            command: "zig",
            options: &["fmt"],
            includes: &["*.zig"],
        }),
    },
    // Terraform
    ToolDef {
        name: "terraform fmt",
        language: "Terraform",
        hook_command: "",
        install_cmd: "brew install terraform",
        check_binary: "terraform",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "terraform-fmt",
            command: "terraform",
            options: &["fmt"],
            includes: &["*.tf", "*.tfvars"],
        }),
    },
    // SQL
    ToolDef {
        name: "sqlfluff fix",
        language: "SQL",
        hook_command: "",
        install_cmd: "pip install sqlfluff",
        check_binary: "sqlfluff",
        category: ToolCategory::Formatter,
        treefmt_entry: Some(TreefmtEntry {
            name: "sqlfluff-fix",
            command: "sqlfluff",
            options: &["fix", "--force"],
            includes: &["*.sql"],
        }),
    },
    // YAML (prettier already covers .yaml via includes)
    // ===================== Linters =====================
    // Rust
    ToolDef {
        name: "clippy",
        language: "Rust",
        hook_command: "cargo clippy -- -D warnings",
        install_cmd: "rustup component add clippy",
        check_binary: "cargo-clippy",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // JS/TS
    ToolDef {
        name: "eslint",
        language: "JS/TS",
        hook_command: "eslint .",
        install_cmd: "npm install -g eslint",
        check_binary: "eslint",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    ToolDef {
        name: "biome lint",
        language: "JS/TS",
        hook_command: "biome lint .",
        install_cmd: "npm install -g @biomejs/biome",
        check_binary: "biome",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // Python
    ToolDef {
        name: "ruff check",
        language: "Python",
        hook_command: "ruff check .",
        install_cmd: "pip install ruff",
        check_binary: "ruff",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    ToolDef {
        name: "mypy",
        language: "Python",
        hook_command: "mypy .",
        install_cmd: "pip install mypy",
        check_binary: "mypy",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    ToolDef {
        name: "pylint",
        language: "Python",
        hook_command: "pylint **/*.py",
        install_cmd: "pip install pylint",
        check_binary: "pylint",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // Go
    ToolDef {
        name: "go vet",
        language: "Go",
        hook_command: "go vet ./...",
        install_cmd: "",
        check_binary: "go",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    ToolDef {
        name: "golangci-lint",
        language: "Go",
        hook_command: "golangci-lint run",
        install_cmd: "brew install golangci-lint",
        check_binary: "golangci-lint",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // C/C++
    ToolDef {
        name: "cppcheck",
        language: "C/C++",
        hook_command: "cppcheck --error-exitcode=1 .",
        install_cmd: "brew install cppcheck",
        check_binary: "cppcheck",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // Java
    ToolDef {
        name: "checkstyle",
        language: "Java",
        hook_command: "checkstyle -c /google_checks.xml src/",
        install_cmd: "brew install checkstyle",
        check_binary: "checkstyle",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // Kotlin
    ToolDef {
        name: "detekt",
        language: "Kotlin",
        hook_command: "detekt --build-upon-default-config",
        install_cmd: "brew install detekt",
        check_binary: "detekt",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // Swift
    ToolDef {
        name: "swiftlint",
        language: "Swift",
        hook_command: "swiftlint lint --strict",
        install_cmd: "brew install swiftlint",
        check_binary: "swiftlint",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // Shell
    ToolDef {
        name: "shellcheck",
        language: "Shell",
        hook_command: "shellcheck **/*.sh",
        install_cmd: "brew install shellcheck",
        check_binary: "shellcheck",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // Lua
    ToolDef {
        name: "luacheck",
        language: "Lua",
        hook_command: "luacheck .",
        install_cmd: "brew install luacheck",
        check_binary: "luacheck",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // PHP
    ToolDef {
        name: "phpstan",
        language: "PHP",
        hook_command: "phpstan analyse",
        install_cmd: "composer global require phpstan/phpstan",
        check_binary: "phpstan",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // SQL
    ToolDef {
        name: "sqlfluff lint",
        language: "SQL",
        hook_command: "sqlfluff lint .",
        install_cmd: "pip install sqlfluff",
        check_binary: "sqlfluff",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // Terraform
    ToolDef {
        name: "tflint",
        language: "Terraform",
        hook_command: "tflint",
        install_cmd: "brew install tflint",
        check_binary: "tflint",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
    // YAML
    ToolDef {
        name: "yamllint",
        language: "YAML",
        hook_command: "yamllint .",
        install_cmd: "pip install yamllint",
        check_binary: "yamllint",
        category: ToolCategory::Linter,
        treefmt_entry: None,
    },
];

/// Installation methods for treefmt itself.
pub struct TreefmtInstallMethod {
    pub label: &'static str,
    pub command: &'static str,
}

pub const TREEFMT_INSTALL_METHODS: &[TreefmtInstallMethod] = &[
    TreefmtInstallMethod {
        label: "Official installer (curl)",
        command: "curl -fsSL https://raw.githubusercontent.com/numtide/treefmt/main/install.sh | bash",
    },
    TreefmtInstallMethod {
        label: "Homebrew",
        command: "brew install treefmt",
    },
    TreefmtInstallMethod {
        label: "Nix",
        command: "nix profile install nixpkgs#treefmt2",
    },
    TreefmtInstallMethod {
        label: "Cargo",
        command: "cargo install treefmt2",
    },
];

pub fn catalog_formatters() -> impl Iterator<Item = &'static ToolDef> {
    TOOL_CATALOG
        .iter()
        .filter(|t| t.category == ToolCategory::Formatter)
}

pub fn catalog_linters() -> impl Iterator<Item = &'static ToolDef> {
    TOOL_CATALOG
        .iter()
        .filter(|t| t.category == ToolCategory::Linter)
}

/// Returns true if the given binary is on the PATH.
pub fn is_binary_on_path(binary: &str) -> bool {
    Command::new("which")
        .arg(binary)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Install a tool with full terminal access. Caller must restore the terminal before calling this.
pub fn install_tool(install_cmd: &str) -> Result<()> {
    let status = Command::new("bash")
        .args(["-c", install_cmd])
        .status()
        .context("failed to run install command")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "install exited with code {}",
            status.code().unwrap_or(-1)
        ))
    }
}

pub enum HookProgress {
    Started {
        name: String,
        index: usize,
        total: usize,
    },
    Passed {
        name: String,
    },
    Failed {
        name: String,
        output: String,
    },
    TimedOut {
        name: String,
        timeout_secs: u64,
    },
    NotFound {
        name: String,
        install_hint: String,
    },
    AllPassed,
}

pub struct HookRunner {
    pub rx: mpsc::Receiver<HookProgress>,
    _handle: thread::JoinHandle<()>,
}

impl HookRunner {
    pub fn start(hooks: Vec<HookEntry>, repo_root: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            run_hooks_seq(&hooks, &repo_root, &tx);
        });
        HookRunner {
            rx,
            _handle: handle,
        }
    }
}

fn run_hooks_seq(hooks: &[HookEntry], repo_root: &Path, tx: &mpsc::Sender<HookProgress>) {
    let total = hooks.len();
    for (i, hook) in hooks.iter().enumerate() {
        if tx
            .send(HookProgress::Started {
                name: hook.name.clone(),
                index: i,
                total,
            })
            .is_err()
        {
            return; // receiver dropped (cancelled)
        }

        let timeout_secs = hook.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let timeout = Duration::from_secs(timeout_secs);

        let result = if let Some(preset_name) = &hook.preset {
            exec_preset(preset_name, repo_root, timeout)
        } else if let Some(cmd_str) = &hook.command {
            exec_custom(cmd_str, repo_root, timeout)
        } else {
            ExecResult::Failed {
                exit_code: None,
                output: "hook has neither preset nor command".to_owned(),
            }
        };

        let progress = match result {
            ExecResult::Passed => HookProgress::Passed {
                name: hook.name.clone(),
            },
            ExecResult::Failed { output, .. } => {
                let _ = tx.send(HookProgress::Failed {
                    name: hook.name.clone(),
                    output,
                });
                return; // short-circuit
            }
            ExecResult::TimedOut => {
                let _ = tx.send(HookProgress::TimedOut {
                    name: hook.name.clone(),
                    timeout_secs,
                });
                return;
            }
            ExecResult::NotFound { install_hint } => {
                let _ = tx.send(HookProgress::NotFound {
                    name: hook.name.clone(),
                    install_hint,
                });
                return;
            }
        };

        if tx.send(progress).is_err() {
            return;
        }
    }
    let _ = tx.send(HookProgress::AllPassed);
}

enum ExecResult {
    Passed,
    Failed {
        #[allow(dead_code)]
        exit_code: Option<i32>,
        output: String,
    },
    TimedOut,
    NotFound {
        install_hint: String,
    },
}

fn exec_preset(name: &str, repo_root: &Path, timeout: Duration) -> ExecResult {
    let Some(preset) = find_preset(name) else {
        return ExecResult::Failed {
            exit_code: None,
            output: format!("unknown preset '{name}'"),
        };
    };

    let program = preset
        .local_paths
        .iter()
        .map(|p| repo_root.join(p))
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(preset.program));

    let mut cmd = Command::new(&program);
    cmd.args(preset.args)
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let result = exec_with_timeout(cmd, timeout);
    if let ExecResult::NotFound { .. } = &result {
        return ExecResult::NotFound {
            install_hint: format!(
                "'{}' not found. Install: {}",
                preset.program, preset.install_url
            ),
        };
    }
    result
}

fn exec_custom(cmd_str: &str, repo_root: &Path, timeout: Duration) -> ExecResult {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", cmd_str])
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    exec_with_timeout(cmd, timeout)
}

fn exec_with_timeout(mut cmd: Command, timeout: Duration) -> ExecResult {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ExecResult::NotFound {
                install_hint: format!("command not found: {e}"),
            };
        }
        Err(e) => {
            return ExecResult::Failed {
                exit_code: None,
                output: format!("failed to spawn: {e}"),
            };
        }
    };

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_thread = thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut pipe) = stdout_pipe {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });
    let stderr_thread = thread::spawn(move || {
        let mut buf = String::new();
        if let Some(mut pipe) = stderr_pipe {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break None,
        }
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();

    match status {
        None => ExecResult::TimedOut,
        Some(s) if s.success() => ExecResult::Passed,
        Some(s) => {
            let mut output = String::new();
            let trimmed_out = stdout.trim();
            let trimmed_err = stderr.trim();
            if !trimmed_out.is_empty() {
                output.push_str(trimmed_out);
            }
            if !trimmed_err.is_empty() {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(trimmed_err);
            }
            ExecResult::Failed {
                exit_code: s.code(),
                output,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_names_matches_presets_constant() {
        let names = preset_names();
        assert_eq!(names.len(), PRESETS.len());
        for (i, name) in names.iter().enumerate() {
            assert_eq!(*name, PRESETS[i].0);
        }
    }

    #[test]
    fn is_known_preset_accepts_valid() {
        assert!(is_known_preset("treefmt"));
    }

    #[test]
    fn is_known_preset_rejects_unknown() {
        assert!(!is_known_preset("unknown_tool"));
    }

    #[test]
    fn spinner_cycles_through_frames() {
        let first = spinner_char(0);
        let second = spinner_char(1);
        assert_ne!(first, second);
        assert_eq!(spinner_char(0), spinner_char(SPINNER_FRAMES.len()));
    }

    #[test]
    fn all_presets_have_valid_definitions() {
        for (name, preset) in PRESETS {
            assert!(!name.is_empty());
            assert!(!preset.program.is_empty());
            assert!(!preset.install_url.is_empty());
        }
    }

    #[test]
    fn find_preset_returns_known_presets() {
        assert!(find_preset("treefmt").is_some());
        assert!(find_preset("unknown").is_none());
    }

    #[test]
    fn tool_catalog_entries_are_valid() {
        for tool in TOOL_CATALOG {
            assert!(!tool.name.is_empty(), "tool name should not be empty");
            assert!(
                !tool.language.is_empty(),
                "tool language should not be empty"
            );
            assert!(
                !tool.check_binary.is_empty(),
                "tool check_binary should not be empty"
            );
            // Formatters must have a treefmt_entry and empty hook_command.
            // Linters must have no treefmt_entry and a non-empty hook_command.
            match tool.category {
                ToolCategory::Formatter => {
                    assert!(
                        tool.treefmt_entry.is_some(),
                        "formatter '{}' must have a treefmt_entry",
                        tool.name
                    );
                    assert!(
                        tool.hook_command.is_empty(),
                        "formatter '{}' must have empty hook_command, got '{}'",
                        tool.name,
                        tool.hook_command
                    );
                }
                ToolCategory::Linter => {
                    assert!(
                        tool.treefmt_entry.is_none(),
                        "linter '{}' must not have a treefmt_entry",
                        tool.name
                    );
                    assert!(
                        !tool.hook_command.is_empty(),
                        "linter '{}' must have a non-empty hook_command",
                        tool.name
                    );
                }
            }
        }
    }

    #[test]
    fn tool_catalog_has_expected_entries() {
        let names: Vec<&str> = TOOL_CATALOG.iter().map(|t| t.name).collect();
        // Formatters — spot check across languages
        assert!(names.contains(&"rustfmt"));
        assert!(names.contains(&"prettier"));
        assert!(names.contains(&"biome format"));
        assert!(names.contains(&"black"));
        assert!(names.contains(&"gofmt"));
        assert!(names.contains(&"gofumpt"));
        assert!(names.contains(&"clang-format"));
        assert!(names.contains(&"shfmt"));
        assert!(names.contains(&"stylua"));
        assert!(names.contains(&"swiftformat"));
        assert!(names.contains(&"terraform fmt"));
        // Linters — spot check across languages
        assert!(names.contains(&"clippy"));
        assert!(names.contains(&"eslint"));
        assert!(names.contains(&"biome lint"));
        assert!(names.contains(&"golangci-lint"));
        assert!(names.contains(&"mypy"));
        assert!(names.contains(&"shellcheck"));
        assert!(names.contains(&"swiftlint"));
        assert!(names.contains(&"tflint"));
        // Old single "biome" should not exist
        assert!(!names.contains(&"biome"));
    }

    #[test]
    fn tool_catalog_formatter_count() {
        let count = catalog_formatters().count();
        assert_eq!(count, 20, "expected 20 formatters in TOOL_CATALOG");
    }

    #[test]
    fn tool_catalog_linter_count() {
        let count = catalog_linters().count();
        assert_eq!(count, 18, "expected 18 linters in TOOL_CATALOG");
    }

    #[test]
    fn treefmt_install_methods_are_valid() {
        for method in TREEFMT_INSTALL_METHODS {
            assert!(
                !method.label.is_empty(),
                "TREEFMT_INSTALL_METHODS entry has empty label"
            );
            assert!(
                !method.command.is_empty(),
                "TREEFMT_INSTALL_METHODS entry '{}' has empty command",
                method.label
            );
        }
    }

    #[test]
    fn catalog_formatters_all_have_treefmt_entry() {
        for tool in catalog_formatters() {
            assert!(
                tool.treefmt_entry.is_some(),
                "formatter '{}' returned by catalog_formatters() must have treefmt_entry",
                tool.name
            );
        }
    }

    #[test]
    fn catalog_linters_none_have_treefmt_entry() {
        for tool in catalog_linters() {
            assert!(
                tool.treefmt_entry.is_none(),
                "linter '{}' returned by catalog_linters() must not have treefmt_entry",
                tool.name
            );
        }
    }

    #[test]
    fn exec_custom_handles_missing_command() {
        let result = exec_custom(
            "nonexistent_command_xyz_12345",
            Path::new("/tmp"),
            Duration::from_secs(5),
        );
        assert!(matches!(result, ExecResult::Failed { .. }));
    }

    #[test]
    fn exec_custom_runs_simple_command() {
        let result = exec_custom("true", Path::new("/tmp"), Duration::from_secs(5));
        assert!(matches!(result, ExecResult::Passed));
    }

    #[test]
    fn exec_custom_captures_failure() {
        let result = exec_custom("false", Path::new("/tmp"), Duration::from_secs(5));
        assert!(matches!(result, ExecResult::Failed { .. }));
    }

    #[test]
    fn exec_with_timeout_kills_slow_command() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let result = exec_with_timeout(cmd, Duration::from_millis(200));
        assert!(matches!(result, ExecResult::TimedOut));
    }
}
