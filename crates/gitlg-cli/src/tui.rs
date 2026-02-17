use std::cmp::{max, min};
use std::collections::HashMap;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use gitlg_core::{CommitSearchQuery, FileChange, GitLgService, GraphData, GraphQuery, GraphRow};
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

#[derive(Debug, Clone)]
pub struct TuiConfig {
    pub repo: PathBuf,
    pub query: GraphQuery,
    pub graph_style: GraphStyle,
    pub max_patch_lines: usize,
    pub git_binary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GraphStyle {
    Unicode,
    Ascii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Commits,
    Files,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitDescPopupMode {
    Generated,
    CommitDone,
    PushDone,
    Error,
}

impl FocusPane {
    fn next(self) -> Self {
        match self {
            Self::Commits => Self::Files,
            Self::Files => Self::Diff,
            Self::Diff => Self::Commits,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Commits => Self::Diff,
            Self::Files => Self::Commits,
            Self::Diff => Self::Files,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Commits => "Commits",
            Self::Files => "Files",
            Self::Diff => "Diff",
        }
    }
}

#[derive(Debug, Default)]
struct CommitArtifactCache {
    files: Arc<Vec<FileChange>>,
    patches: HashMap<String, Vec<Line<'static>>>,
}

pub fn run(service: &GitLgService, config: TuiConfig) -> Result<()> {
    if let Err(err) = super::ensure_gitgraph_local_config(&config.repo) {
        eprintln!(
            "warning: failed to prepare {}: {}",
            config.repo.display(),
            err
        );
    }
    let mut app = TuiApp::new(service, config)?;
    let mut terminal = setup_terminal().context("failed to initialize terminal")?;
    let poll_rate = Duration::from_millis(33);
    let mut needs_redraw = true;

    let run_result = loop {
        if needs_redraw {
            terminal
                .draw(|f| app.draw(f))
                .context("failed to draw TUI frame")?;
            needs_redraw = false;
        }

        if event::poll(poll_rate).context("failed to poll terminal events")? {
            let mut should_quit = false;
            loop {
                match event::read().context("failed to read terminal event")? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if !app.on_key(key)? {
                            should_quit = true;
                            break;
                        }
                        needs_redraw = true;
                    }
                    Event::Mouse(mouse) => {
                        if app.on_mouse(mouse) {
                            needs_redraw = true;
                        }
                    }
                    Event::Resize(_, _) => needs_redraw = true,
                    _ => {}
                }

                if !event::poll(Duration::from_millis(0))
                    .context("failed to poll queued terminal events")?
                {
                    break;
                }
            }
            if should_quit {
                break Ok(());
            }
        } else if app.on_tick()? {
            needs_redraw = true;
        }
    };

    let restore_result = restore_terminal(terminal);
    run_result.and(restore_result)
}

struct TuiApp<'a> {
    service: &'a GitLgService,
    repo: PathBuf,
    git_binary: String,
    base_query: GraphQuery,
    graph: GraphData,
    filtered_rows: Vec<GraphRow>,
    list_state: ListState,
    status: String,
    input_mode: InputMode,
    focus: FocusPane,
    graph_style: GraphStyle,
    max_patch_lines: usize,
    search_input: String,
    list_cache: Vec<Line<'static>>,
    commit_cache: HashMap<String, CommitArtifactCache>,
    current_commit_hash: Option<String>,
    current_files: Arc<Vec<FileChange>>,
    file_list_state: ListState,
    diff_scroll: usize,
    commit_list_area: Option<Rect>,
    file_list_area: Option<Rect>,
    diff_area: Option<Rect>,
    search_area: Option<Rect>,
    pending_commit_prefetch: bool,
    last_commit_change_at: Instant,
    pending_search_apply: bool,
    last_search_input_change_at: Instant,
    commit_desc_popup_lines: Option<Vec<Line<'static>>>,
    commit_desc_popup_mode: Option<CommitDescPopupMode>,
    commit_desc_popup_scroll: usize,
    commit_desc_popup_area: Option<Rect>,
    last_generated_commit_desc: Option<String>,
}

const LANE_COLORS: [Color; 8] = [
    Color::Cyan,
    Color::LightBlue,
    Color::LightGreen,
    Color::Yellow,
    Color::Magenta,
    Color::LightRed,
    Color::LightCyan,
    Color::White,
];
const COMMIT_PREFETCH_DEBOUNCE: Duration = Duration::from_millis(120);
const SEARCH_APPLY_DEBOUNCE: Duration = Duration::from_millis(180);
const MOUSE_DIFF_SCROLL_LINES: i16 = 4;
const COMMIT_DESC_POPUP_SCROLL_LINES: i16 = 4;

impl<'a> TuiApp<'a> {
    fn new(service: &'a GitLgService, config: TuiConfig) -> Result<Self> {
        let graph = service
            .graph(&config.repo, &config.query)
            .with_context(|| format!("failed to load graph for {}", config.repo.display()))?;
        let filtered_rows = graph.commits.clone();
        let mut list_state = ListState::default();
        if !filtered_rows.is_empty() {
            list_state.select(Some(0));
        }

        let mut app = Self {
            service,
            repo: config.repo,
            git_binary: config.git_binary,
            base_query: config.query,
            graph,
            filtered_rows,
            list_state,
            status: "Ready".to_string(),
            input_mode: InputMode::Normal,
            focus: FocusPane::Commits,
            graph_style: config.graph_style,
            max_patch_lines: config.max_patch_lines,
            search_input: String::new(),
            list_cache: Vec::new(),
            commit_cache: HashMap::new(),
            current_commit_hash: None,
            current_files: Arc::new(Vec::new()),
            file_list_state: ListState::default(),
            diff_scroll: 0,
            commit_list_area: None,
            file_list_area: None,
            diff_area: None,
            search_area: None,
            pending_commit_prefetch: false,
            last_commit_change_at: Instant::now(),
            pending_search_apply: false,
            last_search_input_change_at: Instant::now(),
            commit_desc_popup_lines: None,
            commit_desc_popup_mode: None,
            commit_desc_popup_scroll: 0,
            commit_desc_popup_area: None,
            last_generated_commit_desc: None,
        };
        app.rebuild_list_cache();
        app.sync_selected_commit_from_cache();
        Ok(app)
    }

    fn on_tick(&mut self) -> Result<bool> {
        let mut needs_redraw = false;
        if self.pending_search_apply
            && self.last_search_input_change_at.elapsed() >= SEARCH_APPLY_DEBOUNCE
        {
            self.pending_search_apply = false;
            self.apply_search(false)?;
            needs_redraw = true;
        }
        if self.pending_commit_prefetch
            && self.last_commit_change_at.elapsed() >= COMMIT_PREFETCH_DEBOUNCE
        {
            self.pending_commit_prefetch = false;
            self.prefetch_selected_commit_content()?;
            needs_redraw = true;
        }
        Ok(needs_redraw)
    }

    fn on_key(&mut self, key: crossterm::event::KeyEvent) -> Result<bool> {
        if self.commit_desc_popup_lines.is_some() {
            return self.handle_commit_desc_popup_key(key);
        }
        match self.input_mode {
            InputMode::Normal => self.handle_normal_mode_key(key),
            InputMode::Search => self.handle_search_mode_key(key),
        }
    }

    fn handle_normal_mode_key(&mut self, key: crossterm::event::KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => return Ok(false),
            KeyCode::Tab => self.set_focus(self.focus.next())?,
            KeyCode::BackTab => self.set_focus(self.focus.prev())?,
            KeyCode::Right => self.set_focus(self.focus.next())?,
            KeyCode::Left => self.set_focus(self.focus.prev())?,
            KeyCode::Down | KeyCode::Char('j') => match self.focus {
                FocusPane::Commits => self.next_commit()?,
                FocusPane::Files => self.next_file()?,
                FocusPane::Diff => self.scroll_diff(3),
            },
            KeyCode::Up | KeyCode::Char('k') => match self.focus {
                FocusPane::Commits => self.prev_commit()?,
                FocusPane::Files => self.prev_file()?,
                FocusPane::Diff => self.scroll_diff(-3),
            },
            KeyCode::PageDown => self.scroll_diff(14),
            KeyCode::PageUp => self.scroll_diff(-14),
            KeyCode::Char('g') => match self.focus {
                FocusPane::Commits => self.goto_top_commit()?,
                FocusPane::Files => self.goto_top_file()?,
                FocusPane::Diff => self.diff_scroll = 0,
            },
            KeyCode::Char('G') => match self.focus {
                FocusPane::Commits => self.goto_bottom_commit()?,
                FocusPane::Files => self.goto_bottom_file()?,
                FocusPane::Diff => self.diff_scroll = self.max_diff_scroll(),
            },
            KeyCode::Char('r') => self.refresh()?,
            KeyCode::Char('m') => {
                if let Err(err) = self.generate_commit_description_popup() {
                    self.show_commit_desc_error(&err);
                }
            }
            KeyCode::Enter => {
                if self.focus == FocusPane::Files {
                    self.ensure_selected_file_patch_loaded()?;
                }
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Search;
                self.status = "Search: type text, Enter apply, Esc cancel".to_string();
            }
            KeyCode::Esc => {
                if !self.search_input.is_empty() {
                    self.search_input.clear();
                    self.apply_search(true)?;
                }
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    }

    fn set_focus(&mut self, focus: FocusPane) -> Result<()> {
        self.focus = focus;
        if matches!(focus, FocusPane::Files | FocusPane::Diff) {
            self.prefetch_selected_commit_content()?;
        }
        Ok(())
    }

    fn handle_search_mode_key(&mut self, key: crossterm::event::KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Enter => {
                self.input_mode = InputMode::Normal;
                self.pending_search_apply = false;
                self.apply_search(true)?;
            }
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                if self.pending_search_apply {
                    self.pending_search_apply = false;
                    self.apply_search(false)?;
                }
                self.status = "Search canceled".to_string();
            }
            KeyCode::Backspace => {
                self.search_input.pop();
                self.queue_search_apply();
            }
            KeyCode::Char(ch) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.search_input.push(ch);
                    self.queue_search_apply();
                }
            }
            _ => {}
        }
        Ok(true)
    }

    fn handle_commit_desc_popup_key(&mut self, key: crossterm::event::KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.commit_desc_popup_lines = None;
                self.commit_desc_popup_mode = None;
                self.commit_desc_popup_scroll = 0;
                self.commit_desc_popup_area = None;
                self.status = "Commit description closed".to_string();
            }
            KeyCode::Char('c')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.commit_desc_popup_mode == Some(CommitDescPopupMode::Generated) =>
            {
                if let Err(err) = self.auto_commit_from_popup() {
                    self.show_commit_desc_error(&err);
                }
            }
            KeyCode::Char('p')
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.commit_desc_popup_mode == Some(CommitDescPopupMode::CommitDone) =>
            {
                if let Err(err) = self.auto_push_from_popup() {
                    self.show_commit_desc_error(&err);
                }
            }
            KeyCode::PageDown => self.scroll_commit_desc_popup(14),
            KeyCode::PageUp => self.scroll_commit_desc_popup(-14),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_commit_desc_popup(3),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_commit_desc_popup(-3),
            KeyCode::Char('g') => self.commit_desc_popup_scroll = 0,
            KeyCode::Char('G') => {
                self.commit_desc_popup_scroll = self.max_commit_desc_popup_scroll()
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(false);
            }
            _ => {}
        }
        Ok(true)
    }

    fn on_mouse(&mut self, mouse: MouseEvent) -> bool {
        if self.commit_desc_popup_lines.is_some() {
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    self.scroll_commit_desc_popup(COMMIT_DESC_POPUP_SCROLL_LINES);
                    return true;
                }
                MouseEventKind::ScrollUp => {
                    self.scroll_commit_desc_popup(-COMMIT_DESC_POPUP_SCROLL_LINES);
                    return true;
                }
                _ => return true,
            }
        }

        if let Some(area) = self.search_area
            && point_in_rect(area, mouse.column, mouse.row)
        {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                self.input_mode = InputMode::Search;
                self.status = "Search: type text, Enter apply, Esc cancel".to_string();
                return true;
            }
        }

        if self.input_mode == InputMode::Search {
            return false;
        }

        if let Some(area) = self.commit_list_inner_area()
            && point_in_rect(area, mouse.column, mouse.row)
        {
            if let Err(err) = self.set_focus(FocusPane::Commits) {
                self.status = format!("focus error: {err}");
            }
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    let _ = self.next_commit();
                    return true;
                }
                MouseEventKind::ScrollUp => {
                    let _ = self.prev_commit();
                    return true;
                }
                MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::Drag(MouseButton::Left) => {
                    let idx = self
                        .list_state
                        .offset()
                        .saturating_add((mouse.row - area.y) as usize);
                    if let Err(err) = self.set_commit_index(idx) {
                        self.status = format!("commit select error: {err}");
                    }
                    return true;
                }
                _ => return false,
            }
        }

        if let Some(area) = self.file_list_inner_area()
            && point_in_rect(area, mouse.column, mouse.row)
        {
            if let Err(err) = self.set_focus(FocusPane::Files) {
                self.status = format!("focus error: {err}");
            }
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    let _ = self.next_file();
                    return true;
                }
                MouseEventKind::ScrollUp => {
                    let _ = self.prev_file();
                    return true;
                }
                MouseEventKind::Down(MouseButton::Left)
                | MouseEventKind::Drag(MouseButton::Left) => {
                    let idx = self
                        .file_list_state
                        .offset()
                        .saturating_add((mouse.row - area.y) as usize);
                    if let Err(err) = self.set_file_index(idx) {
                        self.status = format!("file select error: {err}");
                    }
                    return true;
                }
                _ => return false,
            }
        }

        if let Some(area) = self.diff_inner_area()
            && point_in_rect(area, mouse.column, mouse.row)
        {
            if self.focus != FocusPane::Diff
                && let Err(err) = self.set_focus(FocusPane::Diff)
            {
                self.status = format!("focus error: {err}");
            }
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    self.scroll_diff(MOUSE_DIFF_SCROLL_LINES);
                    return true;
                }
                MouseEventKind::ScrollUp => {
                    self.scroll_diff(-MOUSE_DIFF_SCROLL_LINES);
                    return true;
                }
                _ => {}
            }
        }

        if let Some(area) = self.diff_area
            && point_in_rect(area, mouse.column, mouse.row)
        {
            if self.focus != FocusPane::Diff
                && let Err(err) = self.set_focus(FocusPane::Diff)
            {
                self.status = format!("focus error: {err}");
            }
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    self.scroll_diff(MOUSE_DIFF_SCROLL_LINES);
                    return true;
                }
                MouseEventKind::ScrollUp => {
                    self.scroll_diff(-MOUSE_DIFF_SCROLL_LINES);
                    return true;
                }
                _ => {}
            }
        }

        false
    }

    fn refresh(&mut self) -> Result<()> {
        self.graph = self
            .service
            .graph(&self.repo, &self.base_query)
            .with_context(|| format!("failed to refresh graph for {}", self.repo.display()))?;
        self.apply_search(true)?;
        self.status = format!("Refreshed {} commit(s)", self.graph.commits.len());
        Ok(())
    }

    fn apply_search(&mut self, prefetch_artifacts: bool) -> Result<()> {
        self.pending_search_apply = false;
        let query = CommitSearchQuery {
            text: self.search_input.clone(),
            ..CommitSearchQuery::default()
        };
        self.filtered_rows = gitlg_core::filter_commits(&self.graph.commits, &query)
            .map_err(|e| anyhow!("search failed: {e}"))?;
        if self.filtered_rows.is_empty() {
            self.list_state.select(None);
            self.clear_current_commit_view();
            self.status = if self.search_input.trim().is_empty() {
                "No commits loaded".to_string()
            } else {
                format!("No matches for {:?}", self.search_input)
            };
            self.rebuild_list_cache();
            return Ok(());
        }

        let selected = self.list_state.selected().unwrap_or(0);
        let bounded = min(selected, self.filtered_rows.len().saturating_sub(1));
        self.list_state.select(Some(bounded));
        self.rebuild_list_cache();
        self.sync_selected_commit_from_cache();
        if prefetch_artifacts {
            self.prefetch_selected_commit_content()?;
        } else {
            self.pending_commit_prefetch = false;
        }
        self.status = if self.search_input.trim().is_empty() {
            format!("Loaded {} commit(s)", self.filtered_rows.len())
        } else {
            format!("Matched {} commit(s)", self.filtered_rows.len())
        };
        Ok(())
    }

    fn next_commit(&mut self) -> Result<()> {
        if self.filtered_rows.is_empty() {
            return Ok(());
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.set_commit_index(min(i + 1, self.filtered_rows.len().saturating_sub(1)))
    }

    fn prev_commit(&mut self) -> Result<()> {
        if self.filtered_rows.is_empty() {
            return Ok(());
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.set_commit_index(i.saturating_sub(1))
    }

    fn goto_top_commit(&mut self) -> Result<()> {
        if self.filtered_rows.is_empty() {
            return Ok(());
        }
        self.set_commit_index(0)
    }

    fn goto_bottom_commit(&mut self) -> Result<()> {
        if self.filtered_rows.is_empty() {
            return Ok(());
        }
        self.set_commit_index(self.filtered_rows.len().saturating_sub(1))
    }

    fn set_commit_index(&mut self, index: usize) -> Result<()> {
        if self.filtered_rows.is_empty() {
            self.list_state.select(None);
            self.clear_current_commit_view();
            return Ok(());
        }
        let bounded = min(index, self.filtered_rows.len().saturating_sub(1));
        self.list_state.select(Some(bounded));
        self.sync_selected_commit_from_cache();
        self.queue_selected_commit_prefetch();
        Ok(())
    }

    fn sync_selected_commit_from_cache(&mut self) {
        let Some(row) = self.selected_row() else {
            self.clear_current_commit_view();
            return;
        };
        let hash = row.hash.clone();
        if self.current_commit_hash.as_deref() == Some(hash.as_str()) {
            return;
        }

        self.current_commit_hash = Some(hash.clone());
        self.file_list_state = ListState::default();
        self.current_files = self
            .commit_cache
            .get(&hash)
            .map(|entry| entry.files.clone())
            .unwrap_or_default();
        self.diff_scroll = 0;
        if self.current_files.is_empty() {
            self.file_list_state.select(None);
        } else {
            self.file_list_state.select(Some(0));
        }
    }

    fn ensure_current_commit_files_loaded(&mut self) -> Result<()> {
        if self.current_commit_hash.is_none() {
            self.sync_selected_commit_from_cache();
        }
        let Some(hash) = self.current_commit_hash.clone() else {
            return Ok(());
        };
        if !self.commit_cache.contains_key(&hash) {
            let files = self
                .service
                .commit_file_changes(&self.repo, &hash)
                .with_context(|| format!("failed to read changed files for commit {hash}"))?;
            self.commit_cache.insert(
                hash.clone(),
                CommitArtifactCache {
                    files: Arc::new(files),
                    patches: HashMap::new(),
                },
            );
        }

        let prev_file_path = self.selected_file_path();
        self.current_files = self
            .commit_cache
            .get(&hash)
            .map(|entry| entry.files.clone())
            .unwrap_or_default();

        if self.current_files.is_empty() {
            self.file_list_state.select(None);
            return Ok(());
        }

        let selected = self
            .file_list_state
            .selected()
            .and_then(|idx| (idx < self.current_files.len()).then_some(idx))
            .or_else(|| {
                prev_file_path.and_then(|path| {
                    self.current_files
                        .iter()
                        .position(|change| change.path == path)
                })
            })
            .unwrap_or(0);
        self.file_list_state.select(Some(selected));
        Ok(())
    }

    fn prefetch_selected_commit_content(&mut self) -> Result<()> {
        self.ensure_current_commit_files_loaded()?;
        if !self.current_files.is_empty() {
            self.ensure_selected_file_patch_loaded()?;
        }
        Ok(())
    }

    fn queue_selected_commit_prefetch(&mut self) {
        self.pending_commit_prefetch = true;
        self.last_commit_change_at = Instant::now();
    }

    fn queue_search_apply(&mut self) {
        self.pending_search_apply = true;
        self.last_search_input_change_at = Instant::now();
    }

    fn generate_commit_description_popup(&mut self) -> Result<()> {
        let text =
            super::generate_commit_description_for_repo_with_git(&self.repo, &self.git_binary)
                .with_context(|| {
                    format!(
                        "failed to generate commit description for {}",
                        self.repo.display()
                    )
                })?;
        let mut lines = text
            .lines()
            .map(|line| {
                Line::from(Span::styled(
                    sanitize_terminal_text(line),
                    Style::default().fg(Color::White),
                ))
            })
            .collect::<Vec<_>>();
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "[c] auto-commit  [Esc/q] close  [PgUp/PgDn/j/k] scroll",
            Style::default().fg(Color::DarkGray),
        )));
        self.commit_desc_popup_lines = Some(lines);
        self.commit_desc_popup_mode = Some(CommitDescPopupMode::Generated);
        self.commit_desc_popup_scroll = 0;
        self.commit_desc_popup_area = None;
        self.last_generated_commit_desc = Some(text);
        self.status = "Commit description generated (press c to auto-commit)".to_string();
        Ok(())
    }

    fn auto_commit_from_popup(&mut self) -> Result<()> {
        let message = self
            .last_generated_commit_desc
            .as_deref()
            .ok_or_else(|| anyhow!("no generated commit description available"))?;
        let result = super::auto_commit_with_message(&self.repo, &self.git_binary, message)
            .with_context(|| format!("failed to auto-commit in {}", self.repo.display()))?;
        let mut lines = vec![
            Line::from(Span::styled(
                "Auto-commit completed",
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::raw("")),
        ];
        lines.extend(result.lines().map(|line| {
            Line::from(Span::styled(
                sanitize_terminal_text(line),
                Style::default().fg(Color::White),
            ))
        }));
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "[p] auto-push  [Esc/q] close",
            Style::default().fg(Color::DarkGray),
        )));
        self.commit_desc_popup_lines = Some(lines);
        self.commit_desc_popup_mode = Some(CommitDescPopupMode::CommitDone);
        self.commit_desc_popup_scroll = 0;
        self.commit_desc_popup_area = None;
        if let Err(err) = self.refresh() {
            self.status = format!("auto-commit done, refresh failed: {err}");
        } else {
            self.status = "Auto-commit completed (press p to auto-push)".to_string();
        }
        Ok(())
    }

    fn auto_push_from_popup(&mut self) -> Result<()> {
        let result = super::auto_push_current_branch(&self.repo, &self.git_binary)
            .with_context(|| format!("failed to auto-push from {}", self.repo.display()))?;
        let mut lines = vec![
            Line::from(Span::styled(
                "Auto-push completed",
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::raw("")),
        ];
        lines.extend(result.lines().map(|line| {
            Line::from(Span::styled(
                sanitize_terminal_text(line),
                Style::default().fg(Color::White),
            ))
        }));
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "[Esc/q] close",
            Style::default().fg(Color::DarkGray),
        )));
        self.commit_desc_popup_lines = Some(lines);
        self.commit_desc_popup_mode = Some(CommitDescPopupMode::PushDone);
        self.commit_desc_popup_scroll = 0;
        self.commit_desc_popup_area = None;
        self.status = "Auto-push completed".to_string();
        Ok(())
    }

    fn show_commit_desc_error(&mut self, err: &anyhow::Error) {
        let causes = err.chain().map(|c| c.to_string()).collect::<Vec<_>>();
        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Commit description error",
            Style::default()
                .fg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::raw("")));
        for (idx, cause) in causes.iter().enumerate() {
            lines.push(Line::from(Span::styled(
                format!("{}. {}", idx + 1, sanitize_terminal_text(cause)),
                Style::default().fg(Color::White),
            )));
        }
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "[Esc/q] close",
            Style::default().fg(Color::DarkGray),
        )));
        self.commit_desc_popup_lines = Some(lines);
        self.commit_desc_popup_mode = Some(CommitDescPopupMode::Error);
        self.commit_desc_popup_scroll = 0;
        self.commit_desc_popup_area = None;
        let root = causes
            .last()
            .cloned()
            .unwrap_or_else(|| "unknown error".to_string());
        self.status = format!("commit desc error: {}", sanitize_terminal_text(&root));
    }

    fn scroll_commit_desc_popup(&mut self, delta: i16) {
        let max_scroll = self.max_commit_desc_popup_scroll();
        if delta > 0 {
            self.commit_desc_popup_scroll = min(
                self.commit_desc_popup_scroll.saturating_add(delta as usize),
                max_scroll,
            );
        } else {
            self.commit_desc_popup_scroll = self
                .commit_desc_popup_scroll
                .saturating_sub((-delta) as usize);
        }
    }

    fn max_commit_desc_popup_scroll(&self) -> usize {
        let total = self.commit_desc_popup_lines.as_ref().map_or(0, Vec::len);
        let visible = self
            .commit_desc_popup_area
            .and_then(inner_block_area)
            .map(|a| a.height as usize)
            .unwrap_or(0);
        total.saturating_sub(visible)
    }

    fn clear_current_commit_view(&mut self) {
        self.current_commit_hash = None;
        self.current_files = Arc::new(Vec::new());
        self.file_list_state = ListState::default();
        self.diff_scroll = 0;
    }

    fn next_file(&mut self) -> Result<()> {
        self.ensure_current_commit_files_loaded()?;
        if self.current_files.is_empty() {
            self.file_list_state.select(None);
            return Ok(());
        }
        let i = self.file_list_state.selected().unwrap_or(0);
        self.set_file_index(min(i + 1, self.current_files.len().saturating_sub(1)))
    }

    fn prev_file(&mut self) -> Result<()> {
        self.ensure_current_commit_files_loaded()?;
        if self.current_files.is_empty() {
            self.file_list_state.select(None);
            return Ok(());
        }
        let i = self.file_list_state.selected().unwrap_or(0);
        self.set_file_index(i.saturating_sub(1))
    }

    fn goto_top_file(&mut self) -> Result<()> {
        self.ensure_current_commit_files_loaded()?;
        if self.current_files.is_empty() {
            self.file_list_state.select(None);
            return Ok(());
        }
        self.set_file_index(0)
    }

    fn goto_bottom_file(&mut self) -> Result<()> {
        self.ensure_current_commit_files_loaded()?;
        if self.current_files.is_empty() {
            self.file_list_state.select(None);
            return Ok(());
        }
        self.set_file_index(self.current_files.len().saturating_sub(1))
    }

    fn set_file_index(&mut self, index: usize) -> Result<()> {
        self.ensure_current_commit_files_loaded()?;
        if self.current_files.is_empty() {
            self.file_list_state.select(None);
            self.diff_scroll = 0;
            return Ok(());
        }
        let bounded = min(index, self.current_files.len().saturating_sub(1));
        self.file_list_state.select(Some(bounded));
        self.diff_scroll = 0;
        Ok(())
    }

    fn ensure_selected_file_patch_loaded(&mut self) -> Result<()> {
        self.ensure_current_commit_files_loaded()?;
        let Some(hash) = self.current_commit_hash.clone() else {
            return Ok(());
        };
        let Some(file_path) = self.selected_file_path() else {
            return Ok(());
        };
        let needs_load = self
            .commit_cache
            .get(&hash)
            .is_none_or(|entry| !entry.patches.contains_key(&file_path));
        if !needs_load {
            return Ok(());
        }

        let patch = self
            .service
            .commit_file_patch(&self.repo, &hash, &file_path, 3)
            .with_context(|| format!("failed to load patch for {file_path} in {hash}"))?;
        let rendered = render_patch_lines(&patch, self.max_patch_lines);
        if let Some(entry) = self.commit_cache.get_mut(&hash) {
            entry.patches.insert(file_path, rendered);
        }
        Ok(())
    }

    fn scroll_diff(&mut self, delta: i16) {
        let max_scroll = self.max_diff_scroll();
        if delta > 0 {
            self.diff_scroll = min(self.diff_scroll.saturating_add(delta as usize), max_scroll);
        } else {
            self.diff_scroll = self.diff_scroll.saturating_sub((-delta) as usize);
        }
    }

    fn max_diff_scroll(&self) -> usize {
        let total = self.current_patch_len();
        let visible = self
            .diff_inner_area()
            .map(|a| a.height as usize)
            .unwrap_or(0);
        total.saturating_sub(visible)
    }

    fn current_patch_len(&self) -> usize {
        self.current_patch_lines().map_or(0, Vec::len)
    }

    fn selected_row(&self) -> Option<&GraphRow> {
        self.list_state
            .selected()
            .and_then(|idx| self.filtered_rows.get(idx))
    }

    fn selected_file(&self) -> Option<&FileChange> {
        self.file_list_state
            .selected()
            .and_then(|idx| self.current_files.get(idx))
    }

    fn selected_file_path(&self) -> Option<String> {
        self.selected_file().map(|f| f.path.clone())
    }

    fn current_patch_lines(&self) -> Option<&Vec<Line<'static>>> {
        let hash = self.current_commit_hash.as_ref()?;
        let file_path = self.selected_file_path()?;
        self.commit_cache.get(hash)?.patches.get(&file_path)
    }

    fn commit_list_inner_area(&self) -> Option<Rect> {
        self.commit_list_area.and_then(inner_block_area)
    }

    fn file_list_inner_area(&self) -> Option<Rect> {
        self.file_list_area.and_then(inner_block_area)
    }

    fn diff_inner_area(&self) -> Option<Rect> {
        self.diff_area.and_then(inner_block_area)
    }

    fn rebuild_list_cache(&mut self) {
        self.list_cache = self
            .filtered_rows
            .iter()
            .map(|row| build_commit_line(row, self.graph_style))
            .collect::<Vec<_>>();
    }

    fn draw(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(8),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(frame.area());

        let header = Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    "GitGraph TUI",
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("repo: {}", self.repo.display()),
                    Style::default().fg(Color::Gray),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("commits: {}", self.filtered_rows.len()),
                    Style::default().fg(Color::White),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("focus: {}", self.focus.as_str()),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
        ]);
        frame.render_widget(header, chunks[0]);

        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(chunks[1]);

        self.commit_list_area = Some(body[0]);
        self.draw_commit_list(frame, body[0]);

        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10),
                Constraint::Length(8),
                Constraint::Min(8),
            ])
            .split(body[1]);
        self.file_list_area = Some(right[1]);
        self.diff_area = Some(right[2]);
        self.draw_details(frame, right[0]);
        self.draw_files(frame, right[1]);
        self.draw_diff(frame, right[2]);

        let search_title = match self.input_mode {
            InputMode::Search => "Search (typing)",
            InputMode::Normal => "Search (/ to edit)",
        };
        let search = Paragraph::new(self.search_input.as_str())
            .block(Block::default().borders(Borders::ALL).title(search_title));
        self.search_area = Some(chunks[2]);
        frame.render_widget(search, chunks[2]);

        let footer = Paragraph::new(format!(
            "{} | q quit | tab switch pane | j/k move | g/G top/bottom | PgUp/PgDn diff | r refresh | m commit-desc | mouse: wheel/click",
            self.status
        ))
        .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(footer, chunks[3]);

        self.draw_commit_desc_popup(frame);
    }

    fn draw_commit_desc_popup(&mut self, frame: &mut Frame) {
        let Some(lines) = self.commit_desc_popup_lines.as_ref() else {
            self.commit_desc_popup_area = None;
            return;
        };
        let area = centered_rect(frame.area(), 86, 72);
        self.commit_desc_popup_area = Some(area);
        let max_scroll = self.max_commit_desc_popup_scroll();
        self.commit_desc_popup_scroll = min(self.commit_desc_popup_scroll, max_scroll);

        let visible_height = inner_block_area(area)
            .map(|inner| inner.height as usize)
            .unwrap_or(0);
        let start = self.commit_desc_popup_scroll;
        let end = min(start.saturating_add(visible_height), lines.len());
        let visible_lines = lines[start..end].to_vec();
        let title = match self
            .commit_desc_popup_mode
            .unwrap_or(CommitDescPopupMode::Generated)
        {
            CommitDescPopupMode::Generated => {
                "Generated Commit Description (c commit, Esc/q close)"
            }
            CommitDescPopupMode::CommitDone => "Auto-commit Done (p push, Esc/q close)",
            CommitDescPopupMode::PushDone => "Auto-push Done (Esc/q close)",
            CommitDescPopupMode::Error => "Commit Description Error (Esc/q close)",
        };

        frame.render_widget(Clear, area);
        let popup = Paragraph::new(Text::from(visible_lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(
                        Style::default()
                            .fg(Color::LightCyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(popup, area);
    }

    fn draw_commit_list(&mut self, frame: &mut Frame, area: Rect) {
        let inner_height = inner_block_area(area)
            .map(|a| a.height as usize)
            .unwrap_or(0);
        let selected = self.list_state.selected().unwrap_or(0);
        let (start, end, selected_local) = visible_window(
            self.list_cache.len(),
            selected,
            self.list_state.offset(),
            inner_height,
        );
        *self.list_state.offset_mut() = start;

        let items = self.list_cache[start..end]
            .iter()
            .cloned()
            .map(ListItem::new);
        let border_style = if self.focus == FocusPane::Commits {
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Commit Graph")
                    .border_style(border_style),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(16, 70, 140))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");
        let mut local_state = ListState::default();
        local_state.select(Some(selected_local));
        frame.render_stateful_widget(list, area, &mut local_state);
    }

    fn draw_details(&self, frame: &mut Frame, area: Rect) {
        let text = if let Some(row) = self.selected_row() {
            let refs = if row.refs.is_empty() {
                "(none)".to_string()
            } else {
                row.refs
                    .iter()
                    .map(|r| r.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let parents = if row.parents.is_empty() {
                "(none)".to_string()
            } else {
                row.parents.join(", ")
            };
            let selected_file = self
                .selected_file()
                .map(|f| f.path.as_str())
                .unwrap_or("(none)");
            let files_loaded = self
                .current_commit_hash
                .as_ref()
                .is_some_and(|hash| self.commit_cache.contains_key(hash));
            let files_changed = if files_loaded {
                self.current_files.len().to_string()
            } else {
                "loading...".to_string()
            };
            let (added_total, removed_total, has_binary_stats) = self.current_files.iter().fold(
                (0u64, 0u64, false),
                |(added_acc, removed_acc, binary), file| {
                    (
                        added_acc + u64::from(file.added.unwrap_or(0)),
                        removed_acc + u64::from(file.removed.unwrap_or(0)),
                        binary || file.added.is_none() || file.removed.is_none(),
                    )
                },
            );
            let diff_totals = if files_loaded {
                if has_binary_stats {
                    format!("+{} / -{} (+bin)", added_total, removed_total)
                } else {
                    format!("+{} / -{}", added_total, removed_total)
                }
            } else {
                "(loading)".to_string()
            };
            format!(
                "hash: {}\nshort: {}\nauthor: {} <{}>\nrefs: {}\nparents: {}\nfiles changed: {}\nchange totals: {}\nselected file: {}\nsubject: {}",
                sanitize_terminal_text(&row.hash),
                sanitize_terminal_text(&row.short_hash),
                sanitize_terminal_text(&row.author_name),
                sanitize_terminal_text(&row.author_email),
                sanitize_terminal_text(&refs),
                sanitize_terminal_text(&parents),
                files_changed,
                diff_totals,
                sanitize_terminal_text(selected_file),
                sanitize_terminal_text(&row.subject)
            )
        } else {
            "No commit selected".to_string()
        };
        let details = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title("Details"))
            .wrap(Wrap { trim: false });
        frame.render_widget(details, area);
    }

    fn draw_files(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focus == FocusPane::Files {
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        if self.current_files.is_empty() {
            let placeholder = if self
                .current_commit_hash
                .as_ref()
                .is_some_and(|hash| !self.commit_cache.contains_key(hash))
            {
                "Files are not loaded yet (switch to Files pane)"
            } else {
                "No file changes for selected commit"
            };
            let files = Paragraph::new(placeholder)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Files")
                        .border_style(border_style),
                )
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(files, area);
            return;
        }

        let inner_height = inner_block_area(area)
            .map(|a| a.height as usize)
            .unwrap_or(0);
        let selected = self.file_list_state.selected().unwrap_or(0);
        let (start, end, selected_local) = visible_window(
            self.current_files.len(),
            selected,
            self.file_list_state.offset(),
            inner_height,
        );
        *self.file_list_state.offset_mut() = start;
        let items = self.current_files[start..end].iter().map(|change| {
            let line = Line::from(vec![
                Span::styled(
                    format!("{:>5}", format_change_count(change.added, '+')),
                    Style::default().fg(Color::Green),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:>5}", format_change_count(change.removed, '-')),
                    Style::default().fg(Color::Red),
                ),
                Span::raw("  "),
                Span::styled(
                    sanitize_terminal_text(&change.path),
                    Style::default().fg(Color::White),
                ),
            ]);
            ListItem::new(line)
        });

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Files (+/-)")
                    .border_style(border_style),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(45, 55, 70))
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");
        let mut local_state = ListState::default();
        local_state.select(Some(selected_local));
        frame.render_stateful_widget(list, area, &mut local_state);
    }

    fn draw_diff(&mut self, frame: &mut Frame, area: Rect) {
        let border_style = if self.focus == FocusPane::Diff {
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let max_scroll = self.max_diff_scroll();
        self.diff_scroll = min(self.diff_scroll, max_scroll);

        let paragraph = if let Some(lines) = self.current_patch_lines() {
            let visible_height = inner_block_area(area)
                .map(|inner| inner.height as usize)
                .unwrap_or(0);
            let start = self.diff_scroll;
            let end = min(start.saturating_add(visible_height), lines.len());
            let visible_lines = lines[start..end].to_vec();
            Paragraph::new(Text::from(visible_lines))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Patch")
                        .border_style(border_style),
                )
                .wrap(Wrap { trim: false })
        } else if self.selected_file().is_some() {
            Paragraph::new("Patch is not loaded yet (Enter on file or open Diff pane)")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Patch")
                        .border_style(border_style),
                )
                .style(Style::default().fg(Color::DarkGray))
        } else {
            Paragraph::new("Select a file to view diff")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Patch")
                        .border_style(border_style),
                )
                .style(Style::default().fg(Color::DarkGray))
        };
        frame.render_widget(paragraph, area);
    }
}

fn build_commit_line(row: &GraphRow, style: GraphStyle) -> Line<'static> {
    let mut spans = build_graph_spans(row, style);
    spans.push(Span::raw("  "));

    let refs = if row.refs.is_empty() {
        String::new()
    } else {
        format!(
            "  [{}]",
            row.refs
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let subject = if row.subject.trim().is_empty() {
        "(no subject)".to_string()
    } else {
        sanitize_terminal_text(&row.subject)
    };
    let author = sanitize_terminal_text(&row.author_name);
    let refs = sanitize_terminal_text(&refs);
    let is_merge = row.parents.len() > 1;

    spans.push(Span::styled(
        format!("{:7}", row.short_hash),
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw("  "));
    if is_merge {
        spans.push(Span::styled(
            "merge ",
            Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(
        subject,
        Style::default().fg(Color::White).add_modifier(if is_merge {
            Modifier::BOLD
        } else {
            Modifier::empty()
        }),
    ));
    spans.push(Span::styled(
        refs,
        Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!("  · {}", author),
        Style::default().fg(Color::DarkGray),
    ));

    Line::from(spans)
}

fn build_graph_spans(row: &GraphRow, style: GraphStyle) -> Vec<Span<'static>> {
    let max_edge_lane = row
        .edges
        .iter()
        .map(|e| e.to_lane)
        .max()
        .unwrap_or(row.lane);
    let lane_count = max(max(row.active_lane_count, row.lane + 1), max_edge_lane + 1);
    let mut out = Vec::with_capacity(lane_count.saturating_mul(2).saturating_add(2));

    let (node_char, line_char, left_edge, right_edge, spacer, connector) = match style {
        GraphStyle::Unicode => ('●', '│', '╱', '╲', '·', '┆'),
        GraphStyle::Ascii => ('o', '|', '/', '\\', ':', '|'),
    };

    for lane in 0..lane_count {
        let color = lane_color(lane);
        let (ch, style) = if lane == row.lane {
            (
                node_char,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )
        } else if row
            .edges
            .iter()
            .any(|e| e.to_lane == lane && e.to_lane != row.lane)
        {
            if lane < row.lane {
                (left_edge, Style::default().fg(color))
            } else {
                (right_edge, Style::default().fg(color))
            }
        } else if lane < row.active_lane_count {
            (
                line_char,
                Style::default().fg(color).add_modifier(Modifier::DIM),
            )
        } else {
            (spacer, Style::default().fg(Color::DarkGray))
        };
        if lane > 0 {
            out.push(Span::styled(" ", Style::default().fg(Color::DarkGray)));
        }
        out.push(Span::styled(ch.to_string(), style));
    }
    if !row.parents.is_empty() {
        out.push(Span::styled(
            connector.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    out
}

fn lane_color(lane: usize) -> Color {
    LANE_COLORS[lane % LANE_COLORS.len()]
}

fn render_patch_lines(patch: &str, max_lines: usize) -> Vec<Line<'static>> {
    if patch.trim().is_empty() {
        return vec![Line::from(Span::styled(
            "(no diff output)",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let unlimited = max_lines == 0;
    let mut lines = Vec::new();
    let mut truncated = false;
    for (idx, raw) in patch.lines().enumerate() {
        if !unlimited && idx >= max_lines {
            truncated = true;
            break;
        }
        let cleaned = sanitize_terminal_text(raw);
        let style = if cleaned.starts_with('+') && !cleaned.starts_with("+++") {
            Style::default().fg(Color::Green)
        } else if cleaned.starts_with('-') && !cleaned.starts_with("---") {
            Style::default().fg(Color::Red)
        } else if cleaned.starts_with("@@") {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if cleaned.starts_with("diff --git")
            || cleaned.starts_with("index ")
            || cleaned.starts_with("--- ")
            || cleaned.starts_with("+++ ")
        {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(Span::styled(cleaned, style)));
    }
    if truncated {
        lines.push(Line::from(Span::styled(
            format!("... truncated to first {max_lines} lines (use narrower diff / higher max)"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}

fn sanitize_terminal_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    let mut in_escape = false;
    let mut in_csi = false;

    while let Some(ch) = chars.next() {
        if in_escape {
            if ch == '[' {
                in_csi = true;
                continue;
            }
            in_escape = false;
            in_csi = false;
            continue;
        }
        if in_csi {
            if ('@'..='~').contains(&ch) {
                in_escape = false;
                in_csi = false;
            }
            continue;
        }

        if ch == '\u{1b}' {
            in_escape = true;
            continue;
        }
        if ch == '\t' {
            out.push_str("    ");
            continue;
        }
        if ch.is_control() {
            continue;
        }
        out.push(ch);
    }
    out
}

fn format_change_count(value: Option<u32>, sign: char) -> String {
    match value {
        Some(v) => format!("{sign}{v}"),
        None => format!("{sign}bin"),
    }
}

fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

fn inner_block_area(area: Rect) -> Option<Rect> {
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    (inner.width > 0 && inner.height > 0).then_some(inner)
}

fn visible_window(
    total: usize,
    selected: usize,
    current_offset: usize,
    visible_height: usize,
) -> (usize, usize, usize) {
    if total == 0 || visible_height == 0 {
        return (0, 0, 0);
    }

    let selected = min(selected, total.saturating_sub(1));
    let mut start = min(current_offset, total.saturating_sub(1));
    if selected < start {
        start = selected;
    }
    let window_last = start.saturating_add(visible_height.saturating_sub(1));
    if selected > window_last {
        start = selected.saturating_sub(visible_height.saturating_sub(1));
    }
    let end = min(start.saturating_add(visible_height), total);
    let selected_local = selected.saturating_sub(start);
    (start, end, selected_local)
}

fn setup_terminal() -> Result<Terminal<ratatui::backend::CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .context("failed to enter alternate screen")?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend).context("failed to create terminal")?;
    Ok(terminal)
}

fn restore_terminal(
    mut terminal: Terminal<ratatui::backend::CrosstermBackend<Stdout>>,
) -> Result<()> {
    disable_raw_mode().context("failed to disable raw mode")?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .context("failed to leave alternate screen")?;
    terminal
        .show_cursor()
        .context("failed to restore cursor visibility")?;
    Ok(())
}

fn point_in_rect(area: Rect, x: u16, y: u16) -> bool {
    x >= area.x
        && x < area.x.saturating_add(area.width)
        && y >= area.y
        && y < area.y.saturating_add(area.height)
}
