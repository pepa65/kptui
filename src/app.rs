use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent};
use keepass::{Database, DatabaseKey};
use ratatui::{
	Frame, Terminal,
	backend::CrosstermBackend,
	layout::{Alignment, Rect},
	style::Style,
	widgets::{Block, ListState, Paragraph, TableState, Wrap},
};
use zeroize::Zeroizing;

use crate::config::{Config, load_config};
use crate::db::Entry;
use crate::input::SECRET_CAP;
use crate::input::edit::{handle_edit_input, process_pending_save};
use crate::input::fieldedit::FieldEditor;
use crate::input::index::handle_index_input;
use crate::input::login::{attempt_unlock, create_database_with_password, database_missing, handle_login_input};
use crate::input::settings::handle_settings_input;
use crate::theme::{Theme, load_theme};
use crate::ui::edit::{EditView, draw_edit};
use crate::ui::index::draw_index;
use crate::ui::login::draw_login;
use crate::ui::settings::draw_settings;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
	Login,
	Index,
	Edit,
	Settings,
}

#[derive(PartialEq, Eq)]
pub enum KeyfileChangeStep {
	Current,
	Path,
}

#[derive(PartialEq, Eq)]
pub enum PasswordChangeStep {
	Current,
	New,
	ConfirmNew,
}

#[derive(PartialEq, Eq)]
pub enum ImportStep {
	Path,
	KdbxPassword,
}

#[derive(PartialEq, Eq)]
pub enum ExportStep {
	Path,
	Confirm,
}

pub struct TotpCache {
	pub code: Zeroizing<String>,
	pub expires: Instant,
}

pub struct App {
	pub screen: Screen,
	pub login_settings: bool,
	pub db_version: Option<String>,
	pub password: Zeroizing<String>,
	pub changing_keyfile: bool,
	pub keyfile_change_step: KeyfileChangeStep,
	pub keyfile_path_buffer: String,
	pub totp_cache: Option<TotpCache>,
	pub query: Zeroizing<String>,
	pub search_notes: bool,
	pub max_len: usize,
	pub index_state: TableState,
	pub index_page_size: usize,
	pub confirm_index_delete: Option<usize>,
	pub edit_state: ListState,
	pub edit_entry: Option<Entry>,
	pub edit_original: Option<Entry>,
	pub edit_target: Option<usize>,
	pub editing_field: bool,
	pub field_editor: Option<FieldEditor>,
	pub field_editor_scroll: usize,
	pub pending_save: bool,
	pub confirm_delete: bool,
	pub confirm_exit: bool,
	pub entries: Vec<Entry>,
	pub filtered: Vec<usize>,
	pub kdbx: Option<Database>,
	pub db_key: Option<DatabaseKey>,
	pub theme: Theme,
	pub config: Config,

	pub creating_database: bool,
	pub confirming_new_db_password: bool,
	pub new_db_confirm: Zeroizing<String>,

	pub available_themes: Vec<String>,
	pub settings_state: ListState,
	pub choosing_theme: bool,
	pub theme_state: ListState,

	pub changing_password: bool,
	pub password_change_step: PasswordChangeStep,
	pub current_password_buffer: Zeroizing<String>,
	pub new_password_buffer: Zeroizing<String>,
	pub new_password_confirm: Zeroizing<String>,

	pub importing_database: bool,
	pub import_step: ImportStep,
	pub import_kdbx_password_buffer: Zeroizing<String>,
	pub pending_import_path: Option<PathBuf>,

	pub exporting_database: bool,
	pub export_step: ExportStep,
	pub pending_export_path: Option<PathBuf>,

	pub login_error: Option<String>,

	pub last_activity: Instant,

	pub status: Option<String>,

	pub should_quit: bool,
	pub slim_mode: bool,

	pub remote: crate::remote::SyncState,
}

impl App {
	fn compute_filtered(&self) -> Vec<usize> {
		let query = Zeroizing::new(self.query.to_lowercase());

		let mut indices: Vec<usize> = self
			.entries
			.iter()
			.enumerate()
			.filter(|(_, entry)| {
				query.is_empty()
					|| Zeroizing::new(entry.name.to_lowercase()).contains(query.as_str())
					|| Zeroizing::new(entry.user.to_lowercase()).contains(query.as_str())
					|| (self.search_notes && Zeroizing::new(entry.notes.to_lowercase()).contains(query.as_str()))
			})
			.map(|(i, _)| i)
			.collect();

		indices.sort_by(|&a, &b| {
			let ea = &self.entries[a];
			let eb = &self.entries[b];
			let a = Zeroizing::new(ea.name.to_lowercase());
			let b = Zeroizing::new(eb.name.to_lowercase());
			a.cmp(&b)
		});

		indices
	}

	pub fn refresh_filter(&mut self) {
		self.filtered = self.compute_filtered();

		let count = self.filtered.len();
		self.index_state.select(if count == 0 { None } else { Some(0) });
	}
}

pub fn maybe_auto_exit(app: &mut App) {
	if !matches!(app.screen, Screen::Index | Screen::Edit | Screen::Settings) {
		return;
	}

	if app.config.auto_exit.is_zero() || app.last_activity.elapsed() < app.config.auto_exit {
		return;
	}

	app.should_quit = true;
}

#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
pub fn start(slim_mode: bool, password: Zeroizing<String>) -> App {
	let mut app = new_app(slim_mode, password);
	open_database(&mut app);
	app
}

/// Sets up the app from the config, without touching the database yet.
fn new_app(slim_mode: bool, password: Zeroizing<String>) -> App {
	let _ = crate::theme::ensure_default_themes();
	let mut config = load_config().unwrap_or_default();
	if config.default_database.is_none() {
		let default_path = crate::util::default_new_database_path();
		if default_path.is_file() {
			config.default_database = Some(default_path);
			let _ = crate::config::save_config(&config);
		}
	}
	let theme = load_theme(config.theme.as_deref()).unwrap_or_default();
	App {
		screen: Screen::Login,
		login_settings: false,
		db_version: None,
		password,
		changing_keyfile: false,
		keyfile_change_step: KeyfileChangeStep::Current,
		keyfile_path_buffer: String::new(),
		totp_cache: None,
		query: Zeroizing::new(String::with_capacity(crate::input::SECRET_CAP)),
		search_notes: false,
		max_len: 0,
		theme,
		config,
		available_themes: Vec::new(),
		settings_state: ListState::default(),
		choosing_theme: false,
		theme_state: ListState::default(),
		changing_password: false,
		password_change_step: PasswordChangeStep::Current,
		current_password_buffer: Zeroizing::new(String::with_capacity(SECRET_CAP)),
		new_password_buffer: Zeroizing::new(String::with_capacity(SECRET_CAP)),
		new_password_confirm: Zeroizing::new(String::with_capacity(SECRET_CAP)),
		importing_database: false,
		import_step: ImportStep::Path,
		import_kdbx_password_buffer: Zeroizing::new(String::with_capacity(SECRET_CAP)),
		pending_import_path: None,
		exporting_database: false,
		export_step: ExportStep::Path,
		pending_export_path: None,
		login_error: None,
		last_activity: Instant::now(),
		index_state: TableState::default().with_selected(Some(0)),
		index_page_size: 1,
		confirm_index_delete: None,
		edit_state: ListState::default(),
		edit_entry: None,
		edit_original: None,
		edit_target: None,
		editing_field: false,
		field_editor: None,
		field_editor_scroll: 0,
		pending_save: false,
		confirm_delete: false,
		confirm_exit: false,
		entries: Vec::new(),
		filtered: Vec::new(),
		kdbx: None,
		db_key: None,
		creating_database: false,
		confirming_new_db_password: false,
		new_db_confirm: Zeroizing::new(String::with_capacity(SECRET_CAP)),
		status: None,
		should_quit: false,
		slim_mode,
		remote: crate::remote::SyncState::default(),
	}
}

/// Updates a remote database's local copy, and unlocks it with a piped-in password.
fn open_database(app: &mut App) {
	crate::remote::open(app);
	app.db_version = app.config.default_database.as_deref().and_then(crate::db::kdbx_version);
	if !app.password.is_empty() {
		if database_missing(app) {
			create_database_with_password(app);
		} else {
			attempt_unlock(app);
		}
	}
	crate::remote::note_writes(app);
}

/// Shown while a remote database gets downloaded, which can take a few seconds.
fn draw_connecting(frame: &mut Frame, app: &App, url: &str) {
	let area = frame.area();
	frame.render_widget(Block::default().style(Style::default().bg(app.theme.background)), area);
	let text = Paragraph::new(format!("Getting {url} ..."))
		.alignment(Alignment::Center)
		.wrap(Wrap { trim: true })
		.style(Style::default().fg(app.theme.warning));
	let middle = Rect { y: area.y + area.height / 2, height: area.height.saturating_sub(area.height / 2), ..area };
	frame.render_widget(text, middle);
}

pub fn draw(frame: &mut Frame, app: &mut App) -> Option<EditView> {
	frame.render_widget(Block::default().style(Style::default().bg(app.theme.background)), frame.area());
	match app.screen {
		Screen::Login => draw_login(frame, app),
		Screen::Index => draw_index(frame, app),
		Screen::Edit => return draw_edit(frame, app),
		Screen::Settings => return draw_settings(frame, app),
	}
	None
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
	app.last_activity = Instant::now();
	match app.screen {
		Screen::Login => handle_login_input(app, key),
		Screen::Index => handle_index_input(app, key),
		Screen::Edit => handle_edit_input(app, key),
		Screen::Settings => handle_settings_input(app, key),
	}
	crate::remote::note_writes(app);
}

pub fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, slim_mode: bool, password: Zeroizing<String>) -> io::Result<()> {
	let mut app = new_app(slim_mode, password);
	if let Some(remote) = app.config.remote.as_ref() {
		let url = remote.url().to_string();
		terminal.draw(|frame| draw_connecting(frame, &app, &url))?;
	}
	open_database(&mut app);
	const TICK_RATE: Duration = Duration::from_millis(200);
	let mut last_scroll = 0usize;
	let mut last_len = 0usize;
	loop {
		terminal.draw(|frame| {
			draw(frame, &mut app);
		})?;
		if app.screen != Screen::Edit {
			last_scroll = 0;
			last_len = 0;
		} else {
			let cur_len = app.field_editor.as_ref().map_or(0, |e| e.text().len());
			let changed = app.field_editor_scroll != last_scroll || cur_len != last_len;
			last_scroll = app.field_editor_scroll;
			last_len = cur_len;
			if changed {
				terminal.clear()?;
				terminal.draw(|frame| {
					draw_edit(frame, &mut app);
				})?;
			}
		}
		if app.pending_save {
			process_pending_save(&mut app);
			continue;
		}
		if app.pending_save {
			process_pending_save(&mut app);
			continue;
		}
		if app.remote.pending {
			crate::remote::process_pending(&mut app);
			continue;
		}

		let has_event = event::poll(TICK_RATE)?;
		maybe_auto_exit(&mut app);
		if app.should_quit {
			break;
		}

		if has_event && let Event::Key(key) = event::read()? {
			handle_key(&mut app, key);
		}

		if app.should_quit {
			break;
		}
	}

	Ok(())
}
