use crate::hooks::ToolCategory;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub status: String,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct RevisionEntry {
    pub commit_id: String,
    pub change_id: String,
    pub author: String,
    pub timestamp: String,
    pub bookmarks: Vec<String>,
    pub description: String,
    pub is_working_copy: bool,
}

#[derive(Debug, Clone)]
pub struct BookmarkEntry {
    pub name: String,
    pub kind: String,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct OperationEntry {
    pub id: String,
    pub is_current: bool,
    pub user: String,
    pub timestamp: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Header,
    Meta,
    Hunk,
    Addition,
    Removal,
    Context,
    Note,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Revisions,
    Bookmarks,
    Operations,
    Diff,
    Output,
}

impl Focus {
    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::Bookmarks,
            Self::Bookmarks => Self::Revisions,
            Self::Revisions => Self::Operations,
            Self::Operations => Self::Diff,
            Self::Diff => Self::Output,
            Self::Output => Self::Files,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Self::Files => Self::Output,
            Self::Bookmarks => Self::Files,
            Self::Revisions => Self::Bookmarks,
            Self::Operations => Self::Revisions,
            Self::Diff => Self::Operations,
            Self::Output => Self::Diff,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Files => "1 Files",
            Self::Bookmarks => "2 Bookmarks",
            Self::Revisions => "3 Revisions",
            Self::Operations => "4 Operations",
            Self::Diff => "5 Diff",
            Self::Output => "6 Command Log",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    DescribeRevision,
    NewChange,
    CreateBookmark,
    MoveBookmark,
    SquashInto,
    RestoreOperation,
    RunCommand,
    ConfirmAbandon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleHelp,
    FocusNext,
    FocusPrevious,
    SetFocus(Focus),
    Refresh,
    InspectCurrent,
    MoveSelection(isize),
    ScrollDiff(isize),
    JumpToStart,
    JumpToEnd,
    OpenPrompt(PromptKind),
    Undo,
    Push,
    Fetch,
    RunHooks,
}

#[derive(Debug, Clone)]
pub struct PromptState {
    pub title: String,
    pub value: String,
    pub placeholder: String,
    pub kind: PromptKind,
}

#[derive(Debug, Clone)]
pub enum HookPhase {
    Running {
        current_hook: String,
        index: usize,
        total: usize,
        spinner_tick: usize,
    },
    Passed {
        count: usize,
        ticks_remaining: usize,
    },
    Failed {
        message: String,
        output: String,
    },
}

#[derive(Debug, Clone)]
pub struct RepoSnapshot {
    pub root: String,
    pub status_summary: Vec<String>,
    pub files: Vec<FileEntry>,
    pub revisions: Vec<RevisionEntry>,
    pub bookmarks: Vec<BookmarkEntry>,
    pub operations: Vec<OperationEntry>,
    pub initial_diff: Vec<DiffLine>,
}

/// One row in the tool picker overlay.
#[derive(Debug, Clone)]
pub struct ToolPickerEntry {
    pub name: String,
    pub language: String,
    pub hook_command: String,
    pub install_cmd: String,
    pub installed: bool,
    pub selected: bool,
    pub category: ToolCategory,
    pub check_binary: String,
}

/// A row in the rendered tool picker list — either a section header or a tool entry.
#[derive(Debug, Clone)]
pub enum ToolPickerRow {
    Header(String),
    Entry(usize),
}

/// State for the interactive tool-picker overlay.
#[derive(Debug, Clone)]
pub struct ToolPickerState {
    pub entries: Vec<ToolPickerEntry>,
    pub cursor: usize,
    pub rows: Vec<ToolPickerRow>,
}

impl ToolPickerState {
    /// Build the rows vec from the current entries.
    /// Emits a Formatters header, then all formatter entries, then a Linters header,
    /// then all linter entries.
    pub fn build_rows(&self) -> Vec<ToolPickerRow> {
        let mut rows = Vec::new();

        // Formatters section
        rows.push(ToolPickerRow::Header("Formatters (via treefmt)".to_owned()));
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.category == ToolCategory::Formatter {
                rows.push(ToolPickerRow::Entry(i));
            }
        }

        // Linters section
        rows.push(ToolPickerRow::Header("Linters".to_owned()));
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.category == ToolCategory::Linter {
                rows.push(ToolPickerRow::Entry(i));
            }
        }

        rows
    }

    /// Returns the index into `rows` of the first `Entry` row.
    pub fn first_tool_row(rows: &[ToolPickerRow]) -> usize {
        rows.iter()
            .position(|r| matches!(r, ToolPickerRow::Entry(_)))
            .unwrap_or(0)
    }

    /// Move the cursor down, skipping header rows.
    pub fn move_cursor_down(&mut self) {
        let mut next = self.cursor + 1;
        while next < self.rows.len() {
            if matches!(self.rows[next], ToolPickerRow::Entry(_)) {
                self.cursor = next;
                return;
            }
            next += 1;
        }
    }

    /// Move the cursor up, skipping header rows.
    pub fn move_cursor_up(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut prev = self.cursor.saturating_sub(1);
        loop {
            if matches!(self.rows[prev], ToolPickerRow::Entry(_)) {
                self.cursor = prev;
                return;
            }
            if prev == 0 {
                return;
            }
            prev -= 1;
        }
    }

    /// Returns the `entries` index that the cursor currently points to, or `None` if
    /// the cursor is on a header.
    pub fn cursor_entry_index(&self) -> Option<usize> {
        match self.rows.get(self.cursor) {
            Some(ToolPickerRow::Entry(idx)) => Some(*idx),
            _ => None,
        }
    }

    /// Returns all entries the user has toggled on.
    pub fn selected_entries(&self) -> Vec<&ToolPickerEntry> {
        self.entries.iter().filter(|e| e.selected).collect()
    }

    /// Returns selected entries that still need to be installed.
    pub fn needs_install(&self) -> Vec<&ToolPickerEntry> {
        self.entries
            .iter()
            .filter(|e| e.selected && !e.installed && !e.install_cmd.is_empty())
            .collect()
    }

    /// Returns selected formatter entries.
    pub fn selected_formatters(&self) -> Vec<&ToolPickerEntry> {
        self.entries
            .iter()
            .filter(|e| e.selected && e.category == ToolCategory::Formatter)
            .collect()
    }

    /// Returns selected linter entries.
    pub fn selected_linters(&self) -> Vec<&ToolPickerEntry> {
        self.entries
            .iter()
            .filter(|e| e.selected && e.category == ToolCategory::Linter)
            .collect()
    }
}

/// State for the treefmt install method picker sub-modal.
#[derive(Debug, Clone, Default)]
pub struct TreefmtInstallState {
    pub cursor: usize,
}

impl TreefmtInstallState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: construct a ToolPickerEntry using the new `category` field.
    // This will fail to compile until `ToolPickerEntry` gains a `category` field
    // and `ToolCategory` is defined.
    fn make_entry(name: &str, category: ToolCategory, selected: bool) -> ToolPickerEntry {
        ToolPickerEntry {
            name: name.to_owned(),
            language: "Test".to_owned(),
            hook_command: if category == ToolCategory::Linter {
                format!("{name} check .")
            } else {
                String::new()
            },
            install_cmd: String::new(),
            installed: true,
            selected,
            category,
            check_binary: String::new(),
        }
    }

    fn make_picker(entries: Vec<ToolPickerEntry>) -> ToolPickerState {
        let rows = {
            // Build rows to set initial cursor
            let mut tmp = ToolPickerState {
                entries: entries.clone(),
                cursor: 0,
                rows: vec![],
            };
            let r = tmp.build_rows();
            tmp.rows = r.clone();
            tmp.cursor = ToolPickerState::first_tool_row(&r);
            r
        };
        ToolPickerState {
            entries,
            cursor: ToolPickerState::first_tool_row(&rows),
            rows,
        }
    }

    #[test]
    fn tool_picker_rows_have_headers() {
        let picker = make_picker(vec![
            make_entry("rustfmt", ToolCategory::Formatter, false),
            make_entry("clippy", ToolCategory::Linter, false),
        ]);

        // `build_rows` does not exist yet — this test will not compile until implemented.
        let rows = picker.build_rows();

        // First row must be the Formatters header
        match &rows[0] {
            ToolPickerRow::Header(label) => {
                assert!(
                    label.contains("Formatter"),
                    "first header should mention Formatters, got '{label}'"
                );
                assert!(
                    label.contains("treefmt"),
                    "Formatters header should mention treefmt, got '{label}'"
                );
            }
            ToolPickerRow::Entry(_) => {
                panic!("expected Header as first row, got Entry");
            }
        }

        // There must also be a Linters header somewhere in the rows
        let linter_header_pos = rows
            .iter()
            .position(|r| matches!(r, ToolPickerRow::Header(l) if l.contains("Linter")));
        assert!(
            linter_header_pos.is_some(),
            "rows must contain a Linters section header"
        );
    }

    #[test]
    fn tool_picker_cursor_skips_headers() {
        // Formatter + two formatters, then a linter, so headers are present between sections.
        let picker = make_picker(vec![
            make_entry("rustfmt", ToolCategory::Formatter, false),
            make_entry("gofmt", ToolCategory::Formatter, false),
            make_entry("clippy", ToolCategory::Linter, false),
        ]);

        // Verify build_rows contains at least one header between entries.
        // `build_rows`, `move_cursor_down`, and `move_cursor_up` don't exist yet.
        let rows = picker.build_rows();
        let has_header = rows.iter().any(|r| matches!(r, ToolPickerRow::Header(_)));
        assert!(has_header, "rows must contain at least one header");

        let mut state = picker;
        // cursor starts at 0 — must be on an Entry row
        for _step in 0..10 {
            let row = state.build_rows();
            assert!(
                matches!(row.get(state.cursor), Some(ToolPickerRow::Entry(_))),
                "cursor at {} points to a Header, not an Entry",
                state.cursor
            );
            let old = state.cursor;
            state.move_cursor_down();
            if state.cursor == old {
                break; // reached the last entry
            }
        }

        // Move back up — must always stay on entries
        for _step in 0..10 {
            let row = state.build_rows();
            assert!(
                matches!(row.get(state.cursor), Some(ToolPickerRow::Entry(_))),
                "cursor at {} points to a Header during upward navigation",
                state.cursor
            );
            let old = state.cursor;
            state.move_cursor_up();
            if state.cursor == old {
                break; // reached the first entry
            }
        }
    }

    #[test]
    fn selected_formatters_returns_only_formatters() {
        let picker = make_picker(vec![
            make_entry("rustfmt", ToolCategory::Formatter, true),
            make_entry("gofmt", ToolCategory::Formatter, false),
            make_entry("clippy", ToolCategory::Linter, true),
            make_entry("eslint", ToolCategory::Linter, false),
        ]);

        // `selected_formatters` does not exist yet.
        let formatters = picker.selected_formatters();
        assert_eq!(
            formatters.len(),
            1,
            "only one formatter is selected; got {:?}",
            formatters.iter().map(|e| &e.name).collect::<Vec<_>>()
        );
        assert_eq!(
            formatters[0].name, "rustfmt",
            "selected formatter should be rustfmt"
        );
        assert_eq!(
            formatters[0].category,
            ToolCategory::Formatter,
            "selected_formatters must only return Formatter entries"
        );

        for f in &formatters {
            assert_ne!(
                f.category,
                ToolCategory::Linter,
                "selected_formatters returned a Linter entry: '{}'",
                f.name
            );
        }
    }

    #[test]
    fn treefmt_install_state_new() {
        // `TreefmtInstallState` does not exist yet.
        let state = TreefmtInstallState::new();
        assert_eq!(
            state.cursor, 0,
            "TreefmtInstallState::new() must start with cursor == 0"
        );
    }
}
