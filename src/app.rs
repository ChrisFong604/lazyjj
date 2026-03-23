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
use crate::hooks::{
    HookProgress, HookRunner, TOOL_CATALOG, TREEFMT_INSTALL_METHODS, catalog_formatters,
    catalog_linters,
};
use crate::jj::JjClient;
use crate::model::{
    Action, DiffKind, DiffLine, FilesContext, Focus, HookPhase, PromptKind, PromptState,
    RepoSnapshot, ToolPickerEntry, ToolPickerState, TreefmtInstallState,
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
    pub files_context: FilesContext,
    pub show_help: bool,
    pub prompt: Option<PromptState>,
    hook_runner: Option<HookRunner>,
    pub hook_phase: Option<HookPhase>,
    push_after_hooks: bool,
    pub tool_picker: Option<ToolPickerState>,
    pub treefmt_install_picker: Option<TreefmtInstallState>,
    /// Install commands pending execution (tool name, install command).
    pub pending_installs: Vec<(String, String)>,
    setup_dismissed: bool,
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
            files_context: FilesContext::WorkingCopy,
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
            tool_picker: None,
            treefmt_install_picker: None,
            pending_installs: vec![],
            setup_dismissed: false,
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
                self.files_context = FilesContext::WorkingCopy;
                self.revisions = snapshot.revisions;
                self.bookmarks = snapshot.bookmarks;
                self.operations = snapshot.operations;
                self.ensure_indices();
                if self.diff_title == "working copy" || self.diff_lines.is_empty() {
                    self.diff_lines = snapshot.initial_diff;
                    self.diff_scroll = 0;
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

    fn update_files_for_revision(&mut self, label: String, revset: String) {
        match self.client.changed_files_for_revision(&revset) {
            Ok(files) => {
                self.files = files;
                self.files_context = FilesContext::Revision { label, revset };
                self.file_index = 0;
            }
            Err(error) => {
                self.push_output(format!("failed to load files: {error}"));
            }
        }
    }

    pub fn files_title(&self) -> String {
        match &self.files_context {
            FilesContext::WorkingCopy => Focus::Files.title().to_owned(),
            FilesContext::Revision { label, .. } => {
                format!("{} ({})", Focus::Files.title(), label)
            }
        }
    }

    /// Full inspect: loads diff AND updates the Files panel for the selected item.
    /// Called on Enter.
    fn inspect_current(&mut self) {
        // Update files for revision/bookmark context (costs 1 jj command)
        match self.focus {
            Focus::Revisions => {
                if let Some(rev) = self.selected_revision().cloned() {
                    self.update_files_for_revision(
                        format!("revision {}", rev.commit_id),
                        rev.commit_id.clone(),
                    );
                }
            }
            Focus::Bookmarks => {
                if let Some(bookmark) = self.selected_bookmark().cloned() {
                    self.update_files_for_revision(
                        format!("bookmark {}", bookmark.name),
                        bookmark.name.clone(),
                    );
                }
            }
            _ => {}
        }
        // Then load the diff (shared with preview)
        self.preview_current();
    }

    /// Lightweight preview: loads only the diff for the selected item (1 jj command).
    /// Called on j/k navigation for responsive auto-preview.
    fn preview_current(&mut self) {
        let result = match self.focus {
            Focus::Files => {
                if let Some(file) = self.selected_file().cloned() {
                    match &self.files_context {
                        FilesContext::WorkingCopy => self.load_diff(
                            format!("file {}", file.path),
                            self.client.diff_for_path(&file.path),
                        ),
                        FilesContext::Revision { revset, .. } => {
                            let revset = revset.clone();
                            self.load_diff(
                                format!("file {}", file.path),
                                self.client.diff_for_revision_path(&revset, &file.path),
                            )
                        }
                    }
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

    /// Build a ToolPickerState from TOOL_CATALOG, checking install status for each tool.
    fn build_tool_picker() -> ToolPickerState {
        let entries: Vec<ToolPickerEntry> = TOOL_CATALOG
            .iter()
            .map(|def| ToolPickerEntry {
                name: def.name.to_owned(),
                language: def.language.to_owned(),
                hook_command: def.hook_command.to_owned(),
                install_cmd: def.install_cmd.to_owned(),
                installed: crate::hooks::is_binary_on_path(def.check_binary),
                selected: false,
                category: def.category,
                check_binary: def.check_binary.to_owned(),
            })
            .collect();
        let mut state = ToolPickerState {
            entries,
            cursor: 0,
            rows: vec![],
        };
        let rows = state.build_rows();
        state.cursor = ToolPickerState::first_tool_row(&rows);
        state.rows = rows;
        state
    }

    fn push(&mut self) {
        if self.hook_phase.is_some() || self.tool_picker.is_some() {
            return;
        }
        let hooks = &self.config.hooks.pre_push;
        if hooks.is_empty() {
            if self.setup_dismissed {
                self.execute_push();
            } else {
                self.push_after_hooks = true;
                self.tool_picker = Some(Self::build_tool_picker());
            }
        } else {
            self.push_after_hooks = true;
            self.start_hooks(hooks.clone());
        }
    }

    /// Handle keystrokes while the tool picker overlay is open.
    /// Returns `true` if installs are needed (caller must handle terminal restore).
    fn on_picker_key(&mut self, key: KeyEvent) -> bool {
        let Some(picker) = &mut self.tool_picker else {
            return false;
        };

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                picker.move_cursor_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                picker.move_cursor_up();
            }
            KeyCode::Char(' ') => {
                if let Some(idx) = picker.cursor_entry_index()
                    && let Some(entry) = picker.entries.get_mut(idx)
                {
                    entry.selected = !entry.selected;
                }
            }
            KeyCode::Enter => {
                // Snapshot what needs to happen before clearing picker
                let needs_install: Vec<(String, String)> = picker
                    .needs_install()
                    .into_iter()
                    .map(|e| (e.name.clone(), e.install_cmd.clone()))
                    .collect();
                let has_selected = !picker.selected_entries().is_empty();
                let has_formatters = !picker.selected_formatters().is_empty();

                if !has_selected {
                    // Nothing selected — dismiss like pressing Esc
                    let should_push = self.push_after_hooks;
                    self.tool_picker = None;
                    self.setup_dismissed = true;
                    self.push_after_hooks = false;
                    if should_push {
                        self.execute_push();
                    }
                    return false;
                }

                // If formatters are selected, check if treefmt is on PATH first
                if has_formatters && !crate::hooks::is_binary_on_path("treefmt") {
                    self.treefmt_install_picker = Some(TreefmtInstallState::new());
                    return false;
                }

                if !needs_install.is_empty() {
                    // Signal the run_loop to handle installs outside the TUI
                    self.pending_installs = needs_install;
                    return true;
                }

                // All selected tools already installed — write config and proceed
                self.enable_selected_tools();
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                let should_push = self.push_after_hooks;
                self.tool_picker = None;
                self.setup_dismissed = true;
                self.push_after_hooks = false;
                if should_push {
                    self.execute_push();
                }
            }
            _ => {}
        }
        false
    }

    /// Handle keystrokes while the treefmt install sub-modal is open.
    /// Returns `true` if installs are needed (caller must handle terminal restore).
    fn on_treefmt_install_key(&mut self, key: KeyEvent) -> bool {
        let Some(state) = &mut self.treefmt_install_picker else {
            return false;
        };

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = TREEFMT_INSTALL_METHODS.len().saturating_sub(1);
                state.cursor = (state.cursor + 1).min(max);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                state.cursor = state.cursor.saturating_sub(1);
            }
            KeyCode::Enter => {
                let cursor = state.cursor;
                // Dismiss sub-modal
                self.treefmt_install_picker = None;

                // Queue treefmt install + all tool installs
                let treefmt_cmd = TREEFMT_INSTALL_METHODS[cursor].command.to_owned();
                let treefmt_label = TREEFMT_INSTALL_METHODS[cursor].label.to_owned();

                let mut installs: Vec<(String, String)> = vec![(treefmt_label, treefmt_cmd)];

                if let Some(picker) = &self.tool_picker {
                    let tool_installs: Vec<(String, String)> = picker
                        .needs_install()
                        .into_iter()
                        .map(|e| (e.name.clone(), e.install_cmd.clone()))
                        .collect();
                    installs.extend(tool_installs);
                }

                self.pending_installs = installs;
                return true;
            }
            KeyCode::Esc => {
                // Dismiss sub-modal, go back to picker
                self.treefmt_install_picker = None;
            }
            _ => {}
        }
        false
    }

    /// Write command hooks for all currently-selected tools, reload config, and start hooks.
    pub fn enable_selected_tools(&mut self) {
        let Some(picker) = self.tool_picker.take() else {
            return;
        };
        let selected = picker.selected_entries();
        if selected.is_empty() {
            return;
        }

        let repo_root = PathBuf::from(&self.repo_root);

        // Formatters: generate treefmt.toml and append a single "treefmt" preset hook
        let selected_formatters = picker.selected_formatters();
        if !selected_formatters.is_empty() {
            let formatter_defs: Vec<&crate::hooks::ToolDef> = catalog_formatters()
                .filter(|def| selected_formatters.iter().any(|e| e.name == def.name))
                .collect();
            if let Err(e) = crate::config::generate_treefmt_toml(&repo_root, &formatter_defs) {
                self.push_output(format!("treefmt.toml generation failed: {e}"));
            }
            let already_has_treefmt = self
                .config
                .hooks
                .pre_push
                .iter()
                .any(|h| h.preset.as_deref() == Some("treefmt"));
            if !already_has_treefmt
                && let Err(e) = crate::config::append_preset_hooks(&repo_root, &["treefmt"])
            {
                self.status_message = Some(format!("Failed to write config: {e}"));
                self.push_output(format!("config write failed: {e}"));
                return;
            }
        }

        // Linters: write command-based hooks using catalog as the source of truth for commands
        let selected_linters = picker.selected_linters();
        let linter_pairs: Vec<(&str, &str)> = catalog_linters()
            .filter(|def| selected_linters.iter().any(|e| e.name == def.name))
            .map(|def| (def.name, def.hook_command))
            .collect();
        if !linter_pairs.is_empty()
            && let Err(e) = crate::config::write_tool_hooks(&repo_root, &linter_pairs)
        {
            self.status_message = Some(format!("Failed to write config: {e}"));
            self.push_output(format!("config write failed: {e}"));
            return;
        }

        match crate::config::load(&repo_root) {
            Ok(cfg) => self.config = cfg,
            Err(e) => {
                self.status_message = Some(format!("Failed to reload config: {e}"));
                self.push_output(format!("config reload failed: {e}"));
                return;
            }
        }

        let names: Vec<&str> = selected.iter().map(|e| e.name.as_str()).collect();
        self.push_output(format!(
            "Enabled tools in .lazyjj.toml: {}",
            names.join(", ")
        ));

        let hooks = self.config.hooks.pre_push.clone();
        self.start_hooks(hooks);
    }

    fn run_hooks_only(&mut self) {
        if self.hook_phase.is_some() || self.tool_picker.is_some() {
            return;
        }
        let hooks = &self.config.hooks.pre_push;
        if hooks.is_empty() {
            self.tool_picker = Some(Self::build_tool_picker());
            return;
        }
        self.push_after_hooks = false;
        self.start_hooks(hooks.clone());
    }

    fn reset_broken_presets(&mut self) {
        let repo = std::path::Path::new(&self.repo_root);
        if let Err(e) = crate::config::remove_preset_hooks(repo) {
            self.push_output(format!("failed to remove preset hooks: {e}"));
            return;
        }
        match crate::config::load(repo) {
            Ok(cfg) => self.config = cfg,
            Err(e) => self.push_output(format!("config reload failed: {e}")),
        }
        self.setup_dismissed = false;
        self.push_output("Removed broken preset hooks from .lazyjj.toml".to_owned());
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
                    self.reset_broken_presets();
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

        if self.treefmt_install_picker.is_some() {
            self.on_treefmt_install_key(key);
            return false;
        }

        if self.tool_picker.is_some() {
            self.on_picker_key(key);
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
                    self.preview_current();
                }
            }
            Focus::Revisions => {
                let prev = self.revision_index;
                adjust_index(&mut self.revision_index, self.revisions.len(), delta);
                if self.revision_index != prev {
                    self.inspect_current();
                }
            }
            Focus::Bookmarks => {
                let prev = self.bookmark_index;
                adjust_index(&mut self.bookmark_index, self.bookmarks.len(), delta);
                if self.bookmark_index != prev {
                    self.inspect_current();
                }
            }
            Focus::Operations => {
                let prev = self.operation_index;
                adjust_index(&mut self.operation_index, self.operations.len(), delta);
                if self.operation_index != prev {
                    self.preview_current();
                }
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
        // Handle tool installs that require full terminal access
        if !app.pending_installs.is_empty() {
            let installs = std::mem::take(&mut app.pending_installs);
            run_tool_installs(terminal, &mut app, installs)?;
            continue;
        }

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

/// Temporarily restore terminal, run installs sequentially, then re-enter TUI
/// and write config for the tools that are now installed.
fn run_tool_installs(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    installs: Vec<(String, String)>,
) -> Result<()> {
    // Leave TUI for the installer
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    let mut failed: Vec<String> = vec![];
    for (name, cmd) in &installs {
        println!("\n--- Installing {name} ---\n");
        match crate::hooks::install_tool(cmd) {
            Ok(()) => {
                app.push_output(format!("✓ {name} installed successfully"));
            }
            Err(e) => {
                eprintln!("✗ {name} install failed: {e}");
                app.push_output(format!("✗ {name} install failed: {e}"));
                failed.push(name.clone());
            }
        }
    }
    println!();

    // Re-enter TUI
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen)?;
    terminal.hide_cursor()?;
    terminal.clear()?;

    // Update install status in picker entries and proceed
    if let Some(picker) = &mut app.tool_picker {
        for entry in &mut picker.entries {
            if entry.selected {
                entry.installed = crate::hooks::is_binary_on_path(&entry.check_binary);
            }
        }
        // Remove entries for tools that failed to install
        for name in &failed {
            if let Some(entry) = picker.entries.iter_mut().find(|e| &e.name == name) {
                entry.selected = false;
            }
        }
    }

    // Write config for tools that are now selected and installed
    app.enable_selected_tools();

    Ok(())
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
    use crate::model::{Action, DiffKind, DiffLine, FilesContext, Focus, PromptKind, RepoSnapshot};

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

    #[test]
    fn tool_picker_cursor_navigation() {
        let mut app = test_app();
        app.tool_picker = Some(App::build_tool_picker());
        let picker = app.tool_picker.as_ref().unwrap();
        let initial_cursor = picker.cursor;
        // With header-aware rows the first row is a Header, so the cursor starts at the
        // first Entry row (index 1 in the rows vec).
        assert!(
            initial_cursor > 0,
            "cursor should start past the first header"
        );
        let first_entry_cursor = initial_cursor;

        // Move down — should advance to the next Entry row
        app.on_picker_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        ));
        let after_down = app.tool_picker.as_ref().unwrap().cursor;
        assert!(
            after_down > first_entry_cursor,
            "cursor should have moved down"
        );

        // Move up — should go back to first entry
        app.on_picker_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('k'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(
            app.tool_picker.as_ref().unwrap().cursor,
            first_entry_cursor,
            "cursor should return to first entry"
        );

        // Cannot go above first entry row
        app.on_picker_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('k'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(
            app.tool_picker.as_ref().unwrap().cursor,
            first_entry_cursor,
            "cursor should not move above first entry row"
        );
    }

    #[test]
    fn tool_picker_space_toggles_selection() {
        let mut app = test_app();
        app.tool_picker = Some(App::build_tool_picker());

        // Toggle on
        app.on_picker_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(app.tool_picker.as_ref().unwrap().entries[0].selected);

        // Toggle off
        app.on_picker_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(!app.tool_picker.as_ref().unwrap().entries[0].selected);
    }

    #[test]
    fn tool_picker_esc_dismisses_without_push() {
        let mut app = test_app();
        app.tool_picker = Some(App::build_tool_picker());
        app.push_after_hooks = false;

        app.on_picker_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));

        assert!(app.tool_picker.is_none());
        assert!(app.setup_dismissed);
    }

    #[test]
    fn build_tool_picker_populates_all_catalog_entries() {
        let picker = App::build_tool_picker();
        assert_eq!(
            picker.entries.len(),
            crate::hooks::TOOL_CATALOG.len(),
            "picker should have one entry per catalog tool"
        );
        for (entry, def) in picker.entries.iter().zip(crate::hooks::TOOL_CATALOG.iter()) {
            assert_eq!(entry.name, def.name);
            assert_eq!(entry.language, def.language);
            assert_eq!(entry.hook_command, def.hook_command);
        }
    }

    // Files context and title tests.

    #[test]
    fn files_title_default_is_working_copy() {
        let app = test_app();
        // When no revision context is set, the Files panel title must equal the
        // static label defined on `Focus::Files`.
        assert_eq!(
            app.files_title(),
            Focus::Files.title(),
            "default files_title() should return the static Focus::Files label"
        );
    }

    #[test]
    fn files_title_includes_revision_label() {
        let mut app = test_app();
        // Switch the Files panel to show a named revision's changed files.
        app.files_context = FilesContext::Revision {
            label: "revision abc".to_owned(),
            revset: "abc".to_owned(),
        };
        let title = app.files_title();
        assert!(
            title.contains("revision abc"),
            "files_title() should contain the revision label 'revision abc', got: '{title}'"
        );
    }

    #[test]
    fn refresh_resets_files_context_to_working_copy() {
        let mut app = test_app();
        // Put the panel in a non-default state first.
        app.files_context = FilesContext::Revision {
            label: "revision abc".to_owned(),
            revset: "abc".to_owned(),
        };
        // A full refresh must bring the Files panel back to showing the working
        // copy, so that the user sees the current state after any jj operation.
        app.refresh();
        assert_eq!(
            app.files_context,
            FilesContext::WorkingCopy,
            "refresh() must reset files_context to FilesContext::WorkingCopy"
        );
    }
}
