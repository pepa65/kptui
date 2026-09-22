use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event};
use keepass::{Database, DatabaseKey};
use ratatui::{
	Terminal,
	backend::CrosstermBackend,
	style::Style,
	widgets::{Block, ListState, TableState},
};
use zeroize::Zeroizing;

use crate::config::{Config, load_config};
use crate::db::Entry;
use crate::input::edit::handle_edit_input;
use crate::input::fieldedit::FieldEditor;
use crate::input::index::handle_index_input;
use crate::input::login::{attempt_unlock, create_database_with_password, database_missing, handle_login_input};
use crate::input::settings::handle_settings_input;
use crate::theme::{Theme, load_theme};
use crate::ui::edit::draw_edit;
use crate::ui::index::draw_index;
use crate::ui::login::draw_login;
use crate::ui::settings::draw_settings;

pub enum Screen {
	Login,
	Index,
	Edit,
	Settings,
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

pub struct App {
	pub screen: Screen,
	pub password: Zeroizing<String>,
	pub query: Zeroizing<String>,
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
	pub import_path_buffer: String,
	pub import_kdbx_password_buffer: Zeroizing<String>,
	pub pending_import_path: Option<PathBuf>,

	pub exporting_database: bool,
	pub export_step: ExportStep,
	pub export_path_buffer: String,
	pub pending_export_path: Option<PathBuf>,

	pub login_error: Option<String>,

	pub last_activity: Instant,

	pub status: Option<String>,

	pub should_quit: bool,
	pub slim_mode: bool,
}

impl App {
	pub fn new(config: Config, theme: Theme, slim_mode: bool, password: Zeroizing<String>) -> Self {
		Self {
			screen: Screen::Login,
			password,
			query: Zeroizing::new(String::new()),
			max_len: 0,
			theme,
			config,
			available_themes: Vec::new(),
			settings_state: ListState::default(),
			choosing_theme: false,
			theme_state: ListState::default(),
			changing_password: false,
			password_change_step: PasswordChangeStep::Current,
			current_password_buffer: Zeroizing::new(String::new()),
			new_password_buffer: Zeroizing::new(String::new()),
			new_password_confirm: Zeroizing::new(String::new()),
			importing_database: false,
			import_step: ImportStep::Path,
			import_path_buffer: String::new(),
			import_kdbx_password_buffer: Zeroizing::new(String::new()),
			pending_import_path: None,
			exporting_database: false,
			export_step: ExportStep::Path,
			export_path_buffer: String::new(),
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
			confirm_delete: false,
			confirm_exit: false,
			entries: Vec::new(),
			filtered: Vec::new(),
			kdbx: None,
			db_key: None,
			creating_database: false,
			confirming_new_db_password: false,
			new_db_confirm: Zeroizing::new(String::new()),
			status: None,
			should_quit: false,
			slim_mode,
		}
	}

	fn compute_filtered(&self) -> Vec<usize> {
		let query = Zeroizing::new(self.query.to_lowercase());
		let mut indices: Vec<usize> = self
			.entries
			.iter()
			.enumerate()
			.filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(query.as_str()) || entry.user.to_lowercase().contains(query.as_str()))
			.map(|(i, _)| i)
			.collect();
		indices.sort_by(|&a, &b| {
			let ea = &self.entries[a];
			let eb = &self.entries[b];
			ea.name.to_lowercase().cmp(&eb.name.to_lowercase())
		});
		indices
	}

	pub fn refresh_filter(&mut self) {
		self.filtered = self.compute_filtered();

		let count = self.filtered.len();
		self.index_state.select(if count == 0 { None } else { Some(0) });
	}
}

fn maybe_auto_lock(app: &mut App) {
	if !matches!(app.screen, Screen::Index | Screen::Edit | Screen::Settings) {
		return;
	}

	if app.last_activity.elapsed() < app.config.auto_lock {
		return;
	}

	app.entries.clear();
	app.filtered.clear();
	app.password.clear();
	app.query.clear();
	app.kdbx = None;
	app.db_key = None;
	app.edit_entry = None;
	app.edit_original = None;
	app.edit_target = None;
	app.editing_field = false;
	app.field_editor = None;
	app.field_editor_scroll = 0;
	app.confirm_delete = false;
	app.confirm_exit = false;
	app.available_themes.clear();
	app.settings_state.select(None);
	app.choosing_theme = false;
	app.theme_state.select(None);
	app.creating_database = false;
	app.confirming_new_db_password = false;
	app.new_db_confirm.clear();
	app.changing_password = false;
	app.password_change_step = PasswordChangeStep::Current;
	app.current_password_buffer.clear();
	app.new_password_buffer.clear();
	app.new_password_confirm.clear();
	app.importing_database = false;
	app.import_step = ImportStep::Path;
	app.import_path_buffer.clear();
	app.import_kdbx_password_buffer.clear();
	app.pending_import_path = None;
	app.exporting_database = false;
	app.export_step = ExportStep::Path;
	app.export_path_buffer.clear();
	app.pending_export_path = None;
	app.screen = Screen::Login;
	app.login_error = Some("Locked after inactivity".to_string());
}

fn load_app_config() -> (Config, Theme) {
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
	(config, theme)
}

fn initialize_database(app: &mut App) {
	if app.password.is_empty() {
		return;
	}

	if database_missing(app) {
		create_database_with_password(app);
	} else {
		attempt_unlock(app);
	}
}
pub fn export(path: &Path, password: Zeroizing<String>) -> anyhow::Result<()> {
	let (config, _theme) = load_app_config();
	let db_path = config.default_database.as_deref().context("No default_database set in configfile")?;

	let (_, _, entries) = crate::db::unlock_database(db_path, &password, config.keyfile.as_deref())?;
	match crate::export::detect_format(path)? {
		crate::export::ExportFormat::Csv => crate::export::export_csv(path, &entries),
		crate::export::ExportFormat::Json => crate::export::export_json(path, &entries),
		crate::export::ExportFormat::Kdbx => crate::db::export_database(path, &password, config.keyfile.as_deref(), &entries),
	}
}

pub fn import(path: &Path, password: Zeroizing<String>, keyfile: Option<&Path>) -> anyhow::Result<()> {
	let (config, _) = load_app_config();
	let database_path = config.default_database.as_deref().context("no default database configured")?;
	let imported = match crate::import::detect_format(path)? {
		crate::import::ImportFormat::Kdbx => crate::import::import_kdbx(path, &password)?,
		crate::import::ImportFormat::Csv => crate::import::import_csv(path)?,
		crate::import::ImportFormat::Json => crate::import::import_json(path)?,
	};
	let (mut db, key, mut entries) = crate::db::unlock_database(database_path, &password, keyfile)?;
	entries.extend(imported);
	crate::db::save_database(database_path, &key, &mut db, &mut entries)
}

pub fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, slim_mode: bool, password: Zeroizing<String>) -> io::Result<()> {
	let (config, theme) = load_app_config();
	let mut app = App::new(config, theme, slim_mode, password);
	initialize_database(&mut app);
	const TICK_RATE: Duration = Duration::from_millis(200);
	loop {
		terminal.draw(|frame| {
			frame.render_widget(Block::default().style(Style::default().bg(app.theme.background)), frame.area());
			match app.screen {
				Screen::Login => draw_login(frame, &mut app),
				Screen::Index => draw_index(frame, &mut app),
				Screen::Edit => draw_edit(frame, &mut app),
				Screen::Settings => draw_settings(frame, &mut app),
			}
		})?;
		if event::poll(TICK_RATE)?
			&& let Event::Key(key) = event::read()?
		{
			app.last_activity = Instant::now();
			match app.screen {
				Screen::Login => handle_login_input(&mut app, key.code),
				Screen::Index => handle_index_input(&mut app, key),
				Screen::Edit => handle_edit_input(&mut app, key),
				Screen::Settings => handle_settings_input(&mut app, key),
			}
		}
		maybe_auto_lock(&mut app);
		if app.should_quit {
			break;
		}
	}

	Ok(())
}
