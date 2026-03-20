use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap,
};

use crate::app::App;
use crate::model::{DiffKind, Focus, PromptState};

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_header(frame, app, root[0]);
    render_body(frame, app, root[1]);
    render_output(frame, app, root[2]);
    render_footer(frame, app, root[3]);

    if let Some(prompt) = &app.prompt {
        render_prompt(frame, prompt);
    } else if app.show_help {
        render_help(frame);
    }
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = vec![
        Span::styled(
            "lazyjj",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(app.repo_root.as_str(), Style::default().fg(Color::Gray)),
        Span::raw("  "),
        Span::styled(
            format!("focus: {}", app.focus.title()),
            Style::default().fg(Color::Yellow),
        ),
    ];
    let summary = if app.status_summary.is_empty() {
        "No repo summary".to_owned()
    } else {
        app.status_summary[0].clone()
    };
    let paragraph = Paragraph::new(Text::from(vec![
        Line::from(title),
        Line::from(Span::styled(summary, Style::default().fg(Color::White))),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Workspace")
            .border_style(Style::default().fg(Color::Blue)),
    );
    frame.render_widget(paragraph, area);
}

fn render_body(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(22),
            Constraint::Percentage(24),
            Constraint::Percentage(54),
        ])
        .split(area);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ])
        .split(columns[0]);

    render_list(
        frame,
        left[0],
        Focus::Files.title(),
        app.focus == Focus::Files,
        &app.file_rows(),
        app.file_index,
    );
    render_list(
        frame,
        left[1],
        Focus::Bookmarks.title(),
        app.focus == Focus::Bookmarks,
        &app.bookmark_rows(),
        app.bookmark_index,
    );
    render_list(
        frame,
        left[2],
        Focus::Revisions.title(),
        app.focus == Focus::Revisions,
        &app.revision_rows(),
        app.revision_index,
    );
    render_list(
        frame,
        columns[1],
        Focus::Operations.title(),
        app.focus == Focus::Operations,
        &app.operation_rows(),
        app.operation_index,
    );
    render_diff(frame, app, columns[2]);
}

fn render_list(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    focused: bool,
    rows: &[Line<'static>],
    selected: usize,
) {
    let items = rows.iter().cloned().map(ListItem::new).collect::<Vec<_>>();

    let block = panel_block(title, focused);
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default().with_selected((!rows.is_empty()).then_some(selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_diff(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let title = format!(
        "{}  +{} -{}  ({})",
        Focus::Diff.title(),
        app.diff_additions,
        app.diff_removals,
        app.diff_title
    );
    let text = Text::from(
        app.diff_lines
            .iter()
            .map(|line| {
                let style = match line.kind {
                    DiffKind::Header => Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    DiffKind::Meta => Style::default().fg(Color::Gray),
                    DiffKind::Hunk => Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                    DiffKind::Addition => Style::default().fg(Color::Green),
                    DiffKind::Removal => Style::default().fg(Color::Red),
                    DiffKind::Context => Style::default().fg(Color::White),
                    DiffKind::Note => Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::ITALIC),
                };
                Line::from(Span::styled(line.text.clone(), style))
            })
            .collect::<Vec<_>>(),
    );
    let paragraph = Paragraph::new(text)
        .block(panel_block(&title, app.focus == Focus::Diff))
        .scroll((app.diff_scroll as u16, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_output(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let lines = app
        .output_log
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .cloned()
        .collect::<Vec<_>>();
    let text = Text::from(
        lines
            .into_iter()
            .rev()
            .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::Gray))))
            .collect::<Vec<_>>(),
    );
    let paragraph = Paragraph::new(text)
        .block(panel_block(
            Focus::Output.title(),
            app.focus == Focus::Output,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let message = app
        .status_message
        .as_ref()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            "Tab focus  j/k move  Enter inspect  r refresh  P push  F fetch  p command  d describe  n new  b bookmark  m move bookmark  s squash  a abandon  u undo  ? help  q quit".to_owned()
        });
    let paragraph = Paragraph::new(message)
        .alignment(Alignment::Left)
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray))
                .padding(Padding::horizontal(1)),
        );
    frame.render_widget(paragraph, area);
}

fn render_prompt(frame: &mut Frame<'_>, prompt: &PromptState) {
    let area = centered_rect(60, 20, frame.area());
    frame.render_widget(Clear, area);
    let content = Text::from(vec![
        Line::from(Span::styled(
            &prompt.title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(Span::raw(if prompt.value.is_empty() {
            format!("{}{}", prompt.placeholder, "...")
        } else {
            prompt.value.clone()
        })),
        Line::raw(""),
        Line::from(Span::styled(
            "Enter submits, Esc cancels",
            Style::default().fg(Color::Gray),
        )),
    ]);
    let widget = Paragraph::new(content)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Action")
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}

fn render_help(frame: &mut Frame<'_>) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);
    let lines = vec![
        "Panel navigation",
        "Tab / Shift+Tab cycle focus",
        "1 Files  2 Bookmarks  3 Revisions  4 Operations  5 Diff  6 Log",
        "",
        "Inspection",
        "Enter opens diff/details for the selected row",
        "j/k or arrows move selection (auto-previews in Files)",
        "J/K scroll the diff viewer",
        "",
        "Remote",
        "P push (selected bookmark or all tracked)",
        "F fetch from default remote",
        "",
        "Mutations",
        "d describe selected revision",
        "n create a new change from the selected revision",
        "b create bookmark at selected revision",
        "m move selected bookmark to selected revision",
        "s squash selected revision into selected destination",
        "a abandon selected revision (type yes to confirm)",
        "u undo last jj operation",
        "o restore selected operation",
        "p run arbitrary jj command",
        "",
        "Refresh and exit",
        "r refresh everything",
        "? toggle this help",
        "q quit",
    ];
    let paragraph = Paragraph::new(lines.join("\n"))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("lazyjj Help")
                .border_style(Style::default().fg(Color::Magenta)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn panel_block(title: &str, focused: bool) -> Block<'static> {
    let border = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    Block::default()
        .borders(Borders::ALL)
        .title(title.to_owned())
        .border_style(Style::default().fg(border))
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup[1])[1]
        .inner(Margin {
            vertical: 0,
            horizontal: 0,
        })
}
