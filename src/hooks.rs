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

pub fn is_treefmt_installed() -> bool {
    Command::new("treefmt")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Install treefmt with full terminal access.
/// Caller must restore the terminal before calling this.
pub fn install_treefmt() -> Result<()> {
    // Try nix first, fall back to cargo
    let status = Command::new("bash")
        .args(["-c", "command -v nix-env >/dev/null 2>&1 && nix-env -iA nixpkgs.treefmt2 || cargo install treefmt2"])
        .status()
        .context("failed to install treefmt")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "treefmt install exited with code {}",
            status.code().unwrap_or(-1)
        ))
    }
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
