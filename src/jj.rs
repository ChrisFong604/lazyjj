use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};

use crate::model::{
    BookmarkEntry, DiffKind, DiffLine, FileEntry, OperationEntry, RepoSnapshot, RevisionEntry,
};

const FIELD_SEP: char = '\u{1f}';

pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
}

pub struct JjClient {
    root: PathBuf,
}

impl JjClient {
    pub fn discover(cwd: &Path) -> Result<Self> {
        let output = Command::new("jj")
            .arg("root")
            .current_dir(cwd)
            .output()
            .context("failed to execute `jj root`")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(anyhow!(
                "could not find a jj repository from {}: {}",
                cwd.display(),
                stderr
            ));
        }
        let root = String::from_utf8(output.stdout)?.trim().to_owned();
        Ok(Self {
            root: PathBuf::from(root),
        })
    }

    pub fn snapshot(&self) -> Result<RepoSnapshot> {
        let status = self.run_read(&["status"])?;
        let files = self.changed_files()?;
        let revisions = self.revisions()?;
        let bookmarks = self.bookmarks()?;
        let operations = self.operations()?;
        let initial_diff = if files.is_empty() {
            self.diff_for_revision("@")?
        } else {
            self.diff_for_path(&files[0].path)?
        };
        Ok(RepoSnapshot {
            root: self.root.display().to_string(),
            status_summary: status
                .stdout
                .lines()
                .map(str::trim_end)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            files,
            revisions,
            bookmarks,
            operations,
            initial_diff,
        })
    }

    pub fn diff_for_path(&self, path: &str) -> Result<Vec<DiffLine>> {
        let output = self.run_read(&["diff", "--git", path])?;
        Ok(parse_diff(&output.stdout))
    }

    pub fn diff_for_revision(&self, revset: &str) -> Result<Vec<DiffLine>> {
        let output = self.run_read(&["show", revset, "--git"])?;
        Ok(parse_diff(&output.stdout))
    }

    pub fn diff_for_operation(&self, op_id: &str) -> Result<Vec<DiffLine>> {
        let output = self.run_read(&["op", "show", op_id])?;
        Ok(parse_diff(&output.stdout))
    }

    pub fn git_push(&self, bookmark: Option<&str>) -> Result<CommandOutput> {
        let mut args = vec!["git", "push"];
        if let Some(name) = bookmark {
            args.push("--bookmark");
            args.push(name);
        }
        self.run(&args)
    }

    pub fn git_fetch(&self) -> Result<CommandOutput> {
        self.run(&["git", "fetch"])
    }

    pub fn run_mutation(&self, args: &[&str]) -> Result<CommandOutput> {
        self.run(args)
    }

    pub fn changed_files(&self) -> Result<Vec<FileEntry>> {
        let output = self.run_read(&["diff", "--summary"])?;
        let files = output
            .stdout
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    return None;
                }
                let mut parts = trimmed.splitn(2, ' ');
                let status = parts.next()?.trim().to_owned();
                let path = parts.next().unwrap_or_default().trim().to_owned();
                Some(FileEntry { status, path })
            })
            .collect();
        Ok(files)
    }

    pub fn revisions(&self) -> Result<Vec<RevisionEntry>> {
        let template = concat_template(&[
            "commit_id",
            "change_id",
            "author.email()",
            "committer.timestamp().local().format(\"%Y-%m-%d %H:%M\")",
            "local_bookmarks.map(|b| b.name()).join(\",\")",
            "description.first_line()",
            "if(current_working_copy, \"true\", \"false\")",
        ]);
        let output = self.run_read(&["log", "-n", "32", "--no-graph", "-T", &template])?;
        let revisions = output
            .stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<_> = line.split(FIELD_SEP).collect();
                if parts.len() < 7 {
                    return None;
                }
                Some(RevisionEntry {
                    commit_id: shorten(parts[0], 12),
                    change_id: shorten(parts[1], 12),
                    author: parts[2].to_owned(),
                    timestamp: parts[3].to_owned(),
                    bookmarks: split_csv(parts[4]),
                    description: empty_fallback(parts[5], "(empty description)"),
                    is_working_copy: parts[6] == "true",
                })
            })
            .collect();
        Ok(revisions)
    }

    pub fn bookmarks(&self) -> Result<Vec<BookmarkEntry>> {
        let template = concat_template(&[
            "name",
            "if(remote, \"remote\", \"local\")",
            "self.normal_target().commit_id()",
        ]);
        let output = self.run_read(&["bookmark", "list", "-T", &template])?;
        let bookmarks = output
            .stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<_> = line.split(FIELD_SEP).collect();
                if parts.len() < 3 {
                    return None;
                }
                Some(BookmarkEntry {
                    name: parts[0].to_owned(),
                    kind: parts[1].to_owned(),
                    target: shorten(parts[2], 12),
                })
            })
            .collect();
        Ok(bookmarks)
    }

    pub fn operations(&self) -> Result<Vec<OperationEntry>> {
        let template = concat_template(&[
            "id",
            "if(current_operation, \"true\", \"false\")",
            "user",
            "time.start().local().format(\"%Y-%m-%d %H:%M\")",
            "description",
        ]);
        let output = self.run_read(&["op", "log", "-n", "24", "--no-graph", "-T", &template])?;
        let operations = output
            .stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<_> = line.split(FIELD_SEP).collect();
                if parts.len() < 5 {
                    return None;
                }
                Some(OperationEntry {
                    id: shorten(parts[0], 14),
                    is_current: parts[1] == "true",
                    user: parts[2].to_owned(),
                    timestamp: parts[3].to_owned(),
                    description: empty_fallback(parts[4], "(no description)"),
                })
            })
            .collect();
        Ok(operations)
    }

    fn run_read(&self, args: &[&str]) -> Result<CommandOutput> {
        self.run(args)
    }

    fn run(&self, args: &[&str]) -> Result<CommandOutput> {
        let output = Command::new("jj")
            .args(args)
            .arg("--color=never")
            .arg("--no-pager")
            .current_dir(&self.root)
            .output()
            .with_context(|| format!("failed to execute `jj {}`", args.join(" ")))?;
        let stdout = String::from_utf8(output.stdout)?;
        let stderr = String::from_utf8(output.stderr)?;
        if output.status.success() {
            Ok(CommandOutput { stdout, stderr })
        } else {
            let message = stderr.trim();
            Err(anyhow!(
                "`jj {}` failed{}",
                args.join(" "),
                if message.is_empty() {
                    String::new()
                } else {
                    format!(": {message}")
                }
            ))
        }
    }
}

fn concat_template(parts: &[&str]) -> String {
    let sep = format!("\"{}\"", FIELD_SEP);
    let joined = parts.join(&format!(" ++ {sep} ++ "));
    format!("{joined} ++ \"\\n\"")
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn empty_fallback(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn shorten(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

fn parse_diff(text: &str) -> Vec<DiffLine> {
    if text.trim().is_empty() {
        return vec![DiffLine {
            kind: DiffKind::Note,
            text: "No diff to display.".to_owned(),
        }];
    }
    text.lines()
        .map(|line| {
            let kind = if line.starts_with("diff --git") || line.starts_with("commit ") {
                DiffKind::Header
            } else if line.starts_with("@@") {
                DiffKind::Hunk
            } else if line.starts_with("+++")
                || line.starts_with("---")
                || line.starts_with("index ")
                || line.starts_with("new file")
                || line.starts_with("deleted file")
                || line.starts_with("Author:")
                || line.starts_with("Change ID:")
                || line.starts_with("Commit ID:")
            {
                DiffKind::Meta
            } else if line.starts_with('+') && !line.starts_with("+++") {
                DiffKind::Addition
            } else if line.starts_with('-') && !line.starts_with("---") {
                DiffKind::Removal
            } else {
                DiffKind::Context
            };
            DiffLine {
                kind,
                text: line.to_owned(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{empty_fallback, parse_diff, split_csv};
    use crate::model::DiffKind;

    #[test]
    fn parse_diff_classifies_lines() {
        let diff = parse_diff(
            "diff --git a/file b/file\nindex 123..456 100644\n@@ -1 +1 @@\n-old\n+new\n context\n",
        );

        assert_eq!(diff[0].kind, DiffKind::Header);
        assert_eq!(diff[1].kind, DiffKind::Meta);
        assert_eq!(diff[2].kind, DiffKind::Hunk);
        assert_eq!(diff[3].kind, DiffKind::Removal);
        assert_eq!(diff[4].kind, DiffKind::Addition);
        assert_eq!(diff[5].kind, DiffKind::Context);
    }

    #[test]
    fn parse_diff_returns_note_for_empty_text() {
        let diff = parse_diff(" \n ");
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].kind, DiffKind::Note);
    }

    #[test]
    fn split_csv_drops_empty_entries() {
        assert_eq!(split_csv("a, b,,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn empty_fallback_uses_fallback_for_blank_values() {
        assert_eq!(empty_fallback("   ", "fallback"), "fallback");
        assert_eq!(empty_fallback("value", "fallback"), "value");
    }
}
