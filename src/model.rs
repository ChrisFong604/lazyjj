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
}

#[derive(Debug, Clone)]
pub struct PromptState {
    pub title: String,
    pub value: String,
    pub placeholder: String,
    pub kind: PromptKind,
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
