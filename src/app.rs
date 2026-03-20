use std::io::{self, Stdout};
use std::panic;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::text::{Line, Span};

use crate::config::Config;
use crate::hooks::{HookProgress, HookRunner};
use crate::jj::JjClient;
use crate::model::{
    Action, DiffKind, DiffLine, Focus, HookPhase, PromptKind, PromptState, RepoSnapshot,
};
use crate::ui;

pub struct App {
    pub repo_root: String,
    client: JjClient,
    pub config: Config,
    pub status_summary: Vec<String>,
    pub files: Vec<crate::model::FileEntry>,
    pub revisions: Vec<crate::model::RevisionEntry>,
    pub bookmarks: Vec<crate::model::BookmarkEntry>,
    pub operations: Vec<crate::model::OperationEntry>,
    pub diff_lines: Vec<DiffLine>,
    pub diff_title: String,
    pub diff_scroll: usize,
    pub diff_additions: usize,
    pub diff_removals: usize,
    pub output_log: Vec<String>,
    pub status_message: Option<String>,
    pub focus: Focus,
    pub file_index: usize,
    pub revision_index: usize,
    pub bookmark_index: usize,
    pub operation_index: usize,
    pub show_help: bool,
    pub prompt: Option<PromptState>,
    hook_runner: Option<HookRunner>,
    pub hook_phase: Option<HookPhase>,
    push_after_hooks: bool,
}

impl App {
    fn new(snapshot: RepoSnapshot, client: JjClient, config: Config) -> Self {
        let mut app = Self {
            repo_root: snapshot.root,
            client,
            config,
            status_summary: snapshot.status_summary,
            files: snapshot.files,
            revisions: snapshot.revisions,
            bookmarks: snapshot.bookmarks,
            operations: snapshot.operations,
            diff_lines: snapshot.initial_diff,
            diff_title: "working copy".to_owned(),
            diff_scroll: 0,
            diff_additions: 0,
            diff_removals: 0,
            output_log: vec!["lazyjj initialized".to_owned()],
            status_message: None,
            focus: Focus::Files,
            file_index: 0,
            revision_index: 0,
            bookmark_index: 0,
            operation_index: 0,
            show_help: false,
            prompt: None,
            hook_runner: None,
            hook_phase: None,
            push_after_hooks: false,
        };
        app.recount_diff();
        app
    }

    pub fn file_rows(&self) -> Vec<Line<'static>> {
        if self.files.is_empty() {
            return vec![Line::from(Span::raw("No changed files"))];
        }
        self.files
            .iter()
            .map(|file| {
                Line::from(vec![
                    Span::styled(
                        format!("{:<2}", file.status),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                    ),
                    Span::raw(" "),
                    Span::raw(file.path.clone()),
                ])
            })
            .collect()
    }

    pub fn revision_rows(&self) -> Vec<Line<'static>> {
        if self.revisions.is_empty() {
            return vec![Line::from(Span::raw("No revisions"))];
        }
        self.revisions
            .iter()
            .map(|rev| {
                let marker = if rev.is_working_copy { "@" } else { " " };
                let bookmarks = if rev.bookmarks.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", rev.bookmarks.join(","))
                };
                Line::from(vec![
                    Span::styled(
                        format!("{marker} {}", rev.commit_id),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
                    ),
                    Span::raw(" "),
                    Span::raw(format!(
                        "{}{}  {}  {}",
                        rev.description, bookmarks, rev.author, rev.timestamp
                    )),
                    Span::styled(
                        format!("  {}", rev.change_id),
                        ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray),
                    ),
                ])
            })
            .collect()
    }

    pub fn bookmark_rows(&self) -> Vec<Line<'static>> {
        if self.bookmarks.is_empty() {
            return vec![Line::from(Span::raw("No bookmarks"))];
        }
        self.bookmarks
            .iter()
            .map(|bookmark| {
                Line::from(vec![
                    Span::styled(
                        bookmark.name.clone(),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Green),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        format!("{} -> {}", bookmark.kind, bookmark.target),
                        ratatui::style::Style::default().fg(ratatatui_color_gray()),
                    ),
                ])
            })
            .collect()
    }

    pub fn operation_rows(&self) -> Vec<Line<'static>> {
        if self.operations.is_empty() {
            return vec![Line::from(Span::raw("No operations"))];
        }
        self.operations
            .iter()
            .map(|op| {
                let marker = if op.is_current { "*" } else { " " };
                Line::from(vec![
                    Span::styled(
                        format!("{marker} {}", op.id),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Magenta),
                    ),
                    Span::raw(" "),
                    Span::raw(format!("{}  {}  {}", op.description, op.user, op.timestamp)),
                ])
            })
            .collect()
    }

    fn refresh(&mut self) {
        match self.client.snapshot() {
            Ok(snapshot) => {
                self.repo_root = snapshot.root;
                self.status_summary = snapshot.status_summary;
                self.files = snapshot.files;
                self.revisions = snapshot.revisions;
                self.bookmarks = snapshot.bookmarks;
                self.operations = snapshot.operations;
                self.ensure_indices();
                if self.diff_title == "working copy" || self.diff_lines.is_empty() {
                    self.diff_lines = snapshot.initial_diff;
                    self.recount_diff();
                }
                self.push_output("refresh complete".to_owned());
                self.status_message = Some("Refreshed repository state".to_owned());
            }
            Err(error) => {
                self.status_message = Some(error.to_string());
                self.push_output(format!("refresh failed: {error}"));
            }
        }
    }

    fn ensure_indices(&mut self) {
        self.file_index = clamp_index(self.file_index, self.files.len());
        self.revision_index = clamp_index(self.revision_index, self.revisions.len());
        self.bookmark_index = clamp_index(self.bookmark_index, self.bookmarks.len());
        self.operation_index = clamp_index(self.operation_index, self.operations.len());
    }

    fn inspect_current(&mut self) {
        let result = match self.focus {
            Focus::Files => {
                if let Some(file) = self.selected_file().cloned() {
                    self.load_diff(
                        format!("file {}", file.path),
                        self.client.diff_for_path(&file.path),
                    )
                } else {
                    Ok(())
                }
            }
            Focus::Revisions => {
                if let Some(rev) = self.selected_revision().cloned() {
                    self.load_diff(
                        format!("revision {}", rev.commit_id),
                        self.client.diff_for_revision(&rev.commit_id),
                    )
                } else {
                    Ok(())
                }
            }
            Focus::Bookmarks => {
                if let Some(bookmark) = self.selected_bookmark().cloned() {
                    self.load_diff(
                        format!("bookmark {}", bookmark.name),
                        self.client.diff_for_revision(&bookmark.name),
                    )
                } else {
                    Ok(())
                }
            }
            Focus::Operations => {
                if let Some(op) = self.selected_operation().cloned() {
                    self.load_diff(
                        format!("operation {}", op.id),
                        self.client.diff_for_operation(&op.id),
                    )
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.status_message = Some(error.to_string());
            self.push_output(format!("inspect failed: {error}"));
        }
    }

    fn load_diff(&mut self, title: String, diff: Result<Vec<DiffLine>>) -> Result<()> {
        self.diff_title = title;
        self.diff_scroll = 0;
        self.diff_lines = diff?;
        self.recount_diff();
        Ok(())
    }

    fn recount_diff(&mut self) {
        self.diff_additions = self
            .diff_lines
            .iter()
            .filter(|line| line.kind == DiffKind::Addition)
            .count();
        self.diff_removals = self
            .diff_lines
            .iter()
            .filter(|line| line.kind == DiffKind::Removal)
            .count();
    }

    fn open_prompt(&mut self, kind: PromptKind, title: &str, placeholder: &str, value: String) {
        self.prompt = Some(PromptState {
            title: title.to_owned(),
            value,
            placeholder: placeholder.to_owned(),
            kind,
        });
    }

    fn submit_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        let value = prompt.value.trim().to_owned();
        let result = match prompt.kind {
            PromptKind::DescribeRevision => self.describe_selected(&value),
            PromptKind::NewChange => self.new_change(&value),
            PromptKind::CreateBookmark => self.create_bookmark(&value),
            PromptKind::MoveBookmark => self.move_bookmark(&value),
            PromptKind::SquashInto => self.squash_selected(&value),
            PromptKind::RestoreOperation => self.restore_operation(&value),
            PromptKind::RunCommand => self.run_freeform(&value),
            PromptKind::ConfirmAbandon => self.confirm_abandon(&value),
        };
        match result {
            Ok(()) => {
                self.refresh();
            }
            Err(error) => {
                self.status_message = Some(error.to_string());
                self.push_output(format!("action failed: {error}"));
            }
        }
    }

    fn describe_selected(&mut self, message: &str) -> Result<()> {
        let revision = self
            .selected_revision_commit()
            .unwrap_or_else(|| "@".to_owned());
        self.run_and_log(
            &["describe", &revision, "-m", message],
            format!("describe {revision}"),
        )
    }

    fn new_change(&mut self, message: &str) -> Result<()> {
        let parent = self
            .selected_revision_commit()
            .unwrap_or_else(|| "@".to_owned());
        self.run_and_log(
            &["new", &parent, "-m", message],
            format!("new change from {parent}"),
        )
    }

    fn create_bookmark(&mut self, name: &str) -> Result<()> {
        let target = self
            .selected_revision_commit()
            .unwrap_or_else(|| "@".to_owned());
        self.run_and_log(
            &["bookmark", "create", name, "-r", &target],
            format!("bookmark create {name} -> {target}"),
        )
    }

    fn move_bookmark(&mut self, target: &str) -> Result<()> {
        let bookmark = self
            .selected_bookmark()
            .context("select a bookmark to move")?
            .name
            .clone();
        self.run_and_log(
            &[
                "bookmark",
                "move",
                &bookmark,
                "--to",
                target,
                "--allow-backwards",
            ],
            format!("bookmark move {bookmark} -> {target}"),
        )
    }

    fn squash_selected(&mut self, destination: &str) -> Result<()> {
        let revision = self
            .selected_revision_commit()
            .context("select a revision to squash")?;
        self.run_and_log(
            &["squash", "--from", &revision, "--into", destination],
            format!("squash {revision} into {destination}"),
        )
    }

    fn restore_operation(&mut self, op_id: &str) -> Result<()> {
        self.run_and_log(
            &["op", "restore", op_id],
            format!("restore operation {op_id}"),
        )
    }

    fn run_freeform(&mut self, args: &str) -> Result<()> {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let segments: Vec<String> = trimmed.split_whitespace().map(ToOwned::to_owned).collect();
        let borrowed = segments.iter().map(String::as_str).collect::<Vec<_>>();
        self.run_and_log(&borrowed, format!("jj {}", trimmed))
    }

    fn confirm_abandon(&mut self, value: &str) -> Result<()> {
        if value != "yes" {
            self.push_output("abandon cancelled".to_owned());
            self.status_message = Some("Type `yes` to abandon the selected revision".to_owned());
            return Ok(());
        }
        let revision = self
            .selected_revision_commit()
            .context("select a revision to abandon")?;
        self.run_and_log(&["abandon", &revision], format!("abandon {revision}"))
    }

    fn push(&mut self) {
        if self.hook_phase.is_some() {
            return;
        }
        let hooks = &self.config.hooks.pre_push;
        if hooks.is_empty() {
            self.execute_push();
        } else {
            self.push_after_hooks = true;
            self.start_hooks(hooks.clone());
        }
    }

    fn run_hooks_only(&mut self) {
        if self.hook_phase.is_some() {
            return;
        }
        let hooks = &self.config.hooks.pre_push;
        if hooks.is_empty() {
            self.status_message =
                Some("No hooks configured — add [[hooks.pre_push]] to .lazyjj.toml".to_owned());
            return;
        }
        self.push_after_hooks = false;
        self.start_hooks(hooks.clone());
    }

    fn start_hooks(&mut self, hooks: Vec<crate::config::HookEntry>) {
        let total = hooks.len();
        let runner = HookRunner::start(hooks, PathBuf::from(&self.repo_root));
        self.hook_runner = Some(runner);
        self.hook_phase = Some(HookPhase::Running {
            current_hook: String::new(),
            index: 0,
            total,
            spinner_tick: 0,
        });
    }

    fn execute_push(&mut self) {
        let bookmark = self.selected_bookmark().map(|b| b.name.clone());
        let result = match &bookmark {
            Some(name) => self.client.git_push(Some(name)),
            None => self.client.git_push(None),
        };
        match result {
            Ok(output) => {
                let label = match &bookmark {
                    Some(name) => format!("push bookmark {name}"),
                    None => "push all".to_owned(),
                };
                self.push_output(format!(
                    "$ jj git push{}",
                    bookmark
                        .as_ref()
                        .map(|n| format!(" --bookmark {n}"))
                        .unwrap_or_default()
                ));
                if !output.stdout.trim().is_empty() {
                    self.push_output(output.stdout.trim().to_owned());
                }
                if !output.stderr.trim().is_empty() {
                    self.push_output(output.stderr.trim().to_owned());
                }
                let prefix = if self.config.hooks.pre_push.is_empty() {
                    ""
                } else {
                    "✓ Hooks passed — "
                };
                self.status_message = Some(format!("{prefix}Executed {label}"));
                self.refresh();
            }
            Err(error) => {
                self.status_message = Some(error.to_string());
                self.push_output(format!("push failed: {error}"));
            }
        }
    }

    pub fn tick(&mut self) {
        // Handle Passed toast countdown (runner already dropped)
        if let Some(HookPhase::Passed {
            ticks_remaining, ..
        }) = &mut self.hook_phase
        {
            if *ticks_remaining == 0 {
                self.hook_phase = None;
                if self.push_after_hooks {
                    self.push_after_hooks = false;
                    self.execute_push();
                } else {
                    self.status_message = Some("✓ All hooks passed".to_owned());
                }
            } else {
                *ticks_remaining -= 1;
            }
            return;
        }

        let Some(runner) = &self.hook_runner else {
            return;
        };

        // Collect messages to avoid borrow conflict
        let messages: Vec<_> = std::iter::from_fn(|| runner.rx.try_recv().ok()).collect();

        for msg in messages {
            match msg {
                HookProgress::Started { name, index, total } => {
                    self.hook_phase = Some(HookPhase::Running {
                        current_hook: name,
                        index,
                        total,
                        spinner_tick: 0,
                    });
                }
                HookProgress::Passed { name } => {
                    self.push_output(format!("✓ Hook '{name}' passed"));
                }
                HookProgress::AllPassed => {
                    let total = match &self.hook_phase {
                        Some(HookPhase::Running { total, .. }) => *total,
                        _ => 0,
                    };
                    self.hook_runner = None;
                    self.hook_phase = Some(HookPhase::Passed {
                        count: total,
                        ticks_remaining: 12, // ~1.5s at 120ms/tick
                    });
                    return;
                }
                HookProgress::Failed { name, output } => {
                    self.push_output(format!("✗ Hook '{name}' failed"));
                    if !output.is_empty() {
                        for line in output.lines() {
                            self.push_output(format!("  {line}"));
                        }
                    }
                    self.hook_runner = None;
                    self.hook_phase = Some(HookPhase::Failed {
                        message: format!("Hook '{name}' failed"),
                        output,
                    });
                    return;
                }
                HookProgress::TimedOut { name, timeout_secs } => {
                    let msg = format!("Hook '{name}' timed out after {timeout_secs}s");
                    self.push_output(format!("✗ {msg}"));
                    self.hook_runner = None;
                    self.hook_phase = Some(HookPhase::Failed {
                        message: msg,
                        output: String::new(),
                    });
                    return;
                }
                HookProgress::NotFound { name, install_hint } => {
                    self.push_output(format!("✗ Hook '{name}': {install_hint}"));
                    self.hook_runner = None;
                    self.hook_phase = Some(HookPhase::Failed {
                        message: format!("Hook '{name}': {install_hint}"),
                        output: String::new(),
                    });
                    return;
                }
            }
        }

        // Advance spinner
        if let Some(HookPhase::Running { spinner_tick, .. }) = &mut self.hook_phase {
            *spinner_tick = spinner_tick.wrapping_add(1);
        }
    }

    fn fetch(&mut self) {
        match self.client.git_fetch() {
            Ok(output) => {
                self.push_output("$ jj git fetch".to_owned());
                if !output.stdout.trim().is_empty() {
                    self.push_output(output.stdout.trim().to_owned());
                }
                if !output.stderr.trim().is_empty() {
                    self.push_output(output.stderr.trim().to_owned());
                }
                self.status_message = Some("Executed fetch".to_owned());
                self.refresh();
            }
            Err(error) => {
                self.status_message = Some(error.to_string());
                self.push_output(format!("fetch failed: {error}"));
            }
        }
    }

    fn undo(&mut self) -> Result<()> {
        self.run_and_log(&["undo"], "undo last operation".to_owned())
    }

    fn run_and_log(&mut self, args: &[&str], label: String) -> Result<()> {
        let output = self.client.run_mutation(args)?;
        self.push_output(format!("$ jj {}", args.join(" ")));
        if !output.stdout.trim().is_empty() {
            self.push_output(output.stdout.trim().to_owned());
        }
        if !output.stderr.trim().is_empty() {
            self.push_output(output.stderr.trim().to_owned());
        }
        self.status_message = Some(format!("Executed {label}"));
        Ok(())
    }

    fn selected_file(&self) -> Option<&crate::model::FileEntry> {
        self.files.get(self.file_index)
    }

    fn selected_revision(&self) -> Option<&crate::model::RevisionEntry> {
        self.revisions.get(self.revision_index)
    }

    fn selected_revision_commit(&self) -> Option<String> {
        self.selected_revision().map(|rev| rev.commit_id.clone())
    }

    fn selected_bookmark(&self) -> Option<&crate::model::BookmarkEntry> {
        self.bookmarks.get(self.bookmark_index)
    }

    fn selected_operation(&self) -> Option<&crate::model::OperationEntry> {
        self.operations.get(self.operation_index)
    }

    fn push_output(&mut self, line: String) {
        self.output_log.push(line);
        if self.output_log.len() > 300 {
            let overflow = self.output_log.len() - 300;
            self.output_log.drain(0..overflow);
        }
    }

    fn selected_revision_target(&self) -> String {
        self.selected_revision_commit()
            .unwrap_or_else(|| "@".to_owned())
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        // Handle hook overlay states first
        if let Some(phase) = &self.hook_phase {
            match phase {
                HookPhase::Running { .. } => match key.code {
                    KeyCode::Char('q') => return true,
                    KeyCode::Esc => {
                        self.hook_runner = None;
                        self.hook_phase = None;
                        self.push_after_hooks = false;
                        self.status_message = Some("Hooks cancelled".to_owned());
                    }
                    _ => {}
                },
                HookPhase::Passed { .. } => {
                    // Any key skips the toast
                    self.hook_phase = None;
                    if self.push_after_hooks {
                        self.push_after_hooks = false;
                        self.execute_push();
                    } else {
                        self.status_message = Some("✓ All hooks passed".to_owned());
                    }
                }
                HookPhase::Failed { .. } => match key.code {
                    KeyCode::Char('q') => return true,
                    KeyCode::Esc | KeyCode::Enter => {
                        self.hook_phase = None;
                    }
                    _ => {}
                },
            }
            return false;
        }

        if let Some(prompt) = &mut self.prompt {
            match key.code {
                KeyCode::Esc => self.prompt = None,
                KeyCode::Enter => self.submit_prompt(),
                KeyCode::Backspace => {
                    prompt.value.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    prompt.value.push(c);
                }
                _ => {}
            }
            return false;
        }

        if let Some(action) = self.action_for_key(key) {
            return self.dispatch(action);
        }
        false
    }

    fn action_for_key(&self, key: KeyEvent) -> Option<Action> {
        let action = match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) => Action::Quit,
            (KeyCode::Char('?'), _) => Action::ToggleHelp,
            (KeyCode::Tab, _) => Action::FocusNext,
            (KeyCode::BackTab, _) => Action::FocusPrevious,
            (KeyCode::Char('1'), _) => Action::SetFocus(Focus::Files),
            (KeyCode::Char('2'), _) => Action::SetFocus(Focus::Bookmarks),
            (KeyCode::Char('3'), _) => Action::SetFocus(Focus::Revisions),
            (KeyCode::Char('4'), _) => Action::SetFocus(Focus::Operations),
            (KeyCode::Char('5'), _) => Action::SetFocus(Focus::Diff),
            (KeyCode::Char('6'), _) => Action::SetFocus(Focus::Output),
            (KeyCode::Char('r'), _) => Action::Refresh,
            (KeyCode::Enter, _) => Action::InspectCurrent,
            (KeyCode::Down, _) | (KeyCode::Char('j'), _) => Action::MoveSelection(1),
            (KeyCode::Up, _) | (KeyCode::Char('k'), _) => Action::MoveSelection(-1),
            (KeyCode::Char('J'), _) => Action::ScrollDiff(1),
            (KeyCode::Char('K'), _) => Action::ScrollDiff(-1),
            (KeyCode::Char('g'), _) => Action::JumpToStart,
            (KeyCode::Char('G'), _) => Action::JumpToEnd,
            (KeyCode::Char('p'), _) => Action::OpenPrompt(PromptKind::RunCommand),
            (KeyCode::Char('d'), _) => Action::OpenPrompt(PromptKind::DescribeRevision),
            (KeyCode::Char('n'), _) => Action::OpenPrompt(PromptKind::NewChange),
            (KeyCode::Char('b'), _) => Action::OpenPrompt(PromptKind::CreateBookmark),
            (KeyCode::Char('m'), _) => Action::OpenPrompt(PromptKind::MoveBookmark),
            (KeyCode::Char('s'), _) => Action::OpenPrompt(PromptKind::SquashInto),
            (KeyCode::Char('a'), _) => Action::OpenPrompt(PromptKind::ConfirmAbandon),
            (KeyCode::Char('u'), _) => Action::Undo,
            (KeyCode::Char('o'), _) => Action::OpenPrompt(PromptKind::RestoreOperation),
            (KeyCode::Char('P'), _) => Action::Push,
            (KeyCode::Char('F'), _) => Action::Fetch,
            (KeyCode::Char('H'), _) => Action::RunHooks,
            _ => return None,
        };
        Some(action)
    }

    fn dispatch(&mut self, action: Action) -> bool {
        match action {
            Action::Quit => return true,
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::FocusNext => self.focus = self.focus.next(),
            Action::FocusPrevious => self.focus = self.focus.previous(),
            Action::SetFocus(focus) => self.focus = focus,
            Action::Refresh => self.refresh(),
            Action::InspectCurrent => self.inspect_current(),
            Action::MoveSelection(delta) => self.move_selection(delta),
            Action::ScrollDiff(delta) => self.scroll_diff(delta),
            Action::JumpToStart => self.jump_to_start(),
            Action::JumpToEnd => self.jump_to_end(),
            Action::OpenPrompt(kind) => self.open_prompt_for(kind),
            Action::Undo => {
                if let Err(error) = self.undo() {
                    self.status_message = Some(error.to_string());
                    self.push_output(format!("undo failed: {error}"));
                } else {
                    self.refresh();
                }
            }
            Action::Push => self.push(),
            Action::Fetch => self.fetch(),
            Action::RunHooks => self.run_hooks_only(),
        }
        false
    }

    fn open_prompt_for(&mut self, kind: PromptKind) {
        match kind {
            PromptKind::RunCommand => self.open_prompt(
                kind,
                "Run arbitrary `jj` subcommand",
                "status --ignore-working-copy",
                String::new(),
            ),
            PromptKind::DescribeRevision => self.open_prompt(
                kind,
                "Describe selected revision",
                "new commit message",
                self.selected_revision()
                    .map(|rev| rev.description.clone())
                    .unwrap_or_default(),
            ),
            PromptKind::NewChange => self.open_prompt(
                kind,
                "Create new change from selected revision",
                "message",
                String::new(),
            ),
            PromptKind::CreateBookmark => self.open_prompt(
                kind,
                "Create bookmark at selected revision",
                "bookmark name",
                String::new(),
            ),
            PromptKind::MoveBookmark => self.open_prompt(
                kind,
                "Move selected bookmark to revision",
                "@",
                self.selected_revision_target(),
            ),
            PromptKind::SquashInto => self.open_prompt(
                kind,
                "Squash selected revision into",
                "destination revset",
                "@".to_owned(),
            ),
            PromptKind::ConfirmAbandon => self.open_prompt(
                kind,
                "Type `yes` to abandon selected revision",
                "yes",
                String::new(),
            ),
            PromptKind::RestoreOperation => self.open_prompt(
                kind,
                "Restore operation id",
                "operation id",
                self.selected_operation()
                    .map(|op| op.id.clone())
                    .unwrap_or_default(),
            ),
        }
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            Focus::Files => {
                let prev = self.file_index;
                adjust_index(&mut self.file_index, self.files.len(), delta);
                if self.file_index != prev {
                    self.inspect_current();
                }
            }
            Focus::Revisions => adjust_index(&mut self.revision_index, self.revisions.len(), delta),
            Focus::Bookmarks => adjust_index(&mut self.bookmark_index, self.bookmarks.len(), delta),
            Focus::Operations => {
                adjust_index(&mut self.operation_index, self.operations.len(), delta)
            }
            Focus::Diff => self.scroll_diff(delta),
            Focus::Output => {}
        }
    }

    fn scroll_diff(&mut self, delta: isize) {
        if delta.is_negative() {
            self.diff_scroll = self.diff_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.diff_scroll = self.diff_scroll.saturating_add(delta as usize);
        }
    }

    fn jump_to_start(&mut self) {
        match self.focus {
            Focus::Files => self.file_index = 0,
            Focus::Revisions => self.revision_index = 0,
            Focus::Bookmarks => self.bookmark_index = 0,
            Focus::Operations => self.operation_index = 0,
            Focus::Diff => self.diff_scroll = 0,
            Focus::Output => {}
        }
    }

    fn jump_to_end(&mut self) {
        match self.focus {
            Focus::Files => self.file_index = self.files.len().saturating_sub(1),
            Focus::Revisions => self.revision_index = self.revisions.len().saturating_sub(1),
            Focus::Bookmarks => self.bookmark_index = self.bookmarks.len().saturating_sub(1),
            Focus::Operations => self.operation_index = self.operations.len().saturating_sub(1),
            Focus::Diff => self.diff_scroll = self.diff_lines.len().saturating_sub(1),
            Focus::Output => {}
        }
    }
}

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir().context("failed to determine current working directory")?;
    let client = JjClient::discover(&cwd)?;
    let snapshot = client.snapshot()?;
    let config =
        crate::config::load(std::path::Path::new(&snapshot.root)).context("config error")?;

    install_panic_hook();
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    let _terminal_guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to initialize terminal")?;

    let app = App::new(snapshot, client, config);
    let result = run_loop(&mut terminal, app);

    restore_terminal(terminal)?;
    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, mut app: App) -> Result<()> {
    loop {
        app.tick();
        terminal.draw(|frame| ui::render(frame, &app))?;
        if event::poll(Duration::from_millis(120))?
            && let Event::Key(key) = event::read()?
            && app.on_key(key)
        {
            return Ok(());
        }
    }
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
    Ok(())
}

fn adjust_index(index: &mut usize, len: usize, delta: isize) {
    if len == 0 {
        *index = 0;
        return;
    }
    let next = (*index as isize + delta).clamp(0, len.saturating_sub(1) as isize);
    *index = next as usize;
}

fn clamp_index(index: usize, len: usize) -> usize {
    if len == 0 { 0 } else { index.min(len - 1) }
}

fn ratatatui_color_gray() -> ratatui::style::Color {
    ratatui::style::Color::Gray
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        cleanup_terminal();
    }
}

fn cleanup_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stderr(), LeaveAlternateScreen);
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        cleanup_terminal();
        default_hook(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::{App, adjust_index, clamp_index};
    use crate::config::Config;
    use crate::jj::JjClient;
    use crate::model::{Action, DiffKind, DiffLine, Focus, PromptKind, RepoSnapshot};

    fn test_app() -> App {
        let client = JjClient::discover(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("test repo should be discoverable");
        App::new(
            RepoSnapshot {
                root: "/tmp".to_owned(),
                status_summary: vec![],
                files: vec![],
                revisions: vec![],
                bookmarks: vec![],
                operations: vec![],
                initial_diff: vec![DiffLine {
                    kind: DiffKind::Context,
                    text: "line".to_owned(),
                }],
            },
            client,
            Config::default(),
        )
    }

    #[test]
    fn adjust_index_clamps_to_bounds() {
        let mut index = 1;
        adjust_index(&mut index, 3, -5);
        assert_eq!(index, 0);
        adjust_index(&mut index, 3, 10);
        assert_eq!(index, 2);
    }

    #[test]
    fn clamp_index_handles_empty_lists() {
        assert_eq!(clamp_index(5, 0), 0);
        assert_eq!(clamp_index(5, 3), 2);
    }

    #[test]
    fn key_actions_map_to_expected_commands() {
        let app = test_app();
        assert_eq!(
            app.action_for_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('q'),
                crossterm::event::KeyModifiers::NONE,
            )),
            Some(Action::Quit)
        );
        assert_eq!(
            app.action_for_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('d'),
                crossterm::event::KeyModifiers::NONE,
            )),
            Some(Action::OpenPrompt(PromptKind::DescribeRevision))
        );
        assert_eq!(
            app.action_for_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Tab,
                crossterm::event::KeyModifiers::NONE,
            )),
            Some(Action::FocusNext)
        );
    }

    #[test]
    fn dispatch_updates_focus() {
        let mut app = test_app();
        assert!(!app.dispatch(Action::SetFocus(Focus::Bookmarks)));
        assert_eq!(app.focus, Focus::Bookmarks);
        assert!(!app.dispatch(Action::FocusNext));
        assert_eq!(app.focus, Focus::Revisions);
    }
}
