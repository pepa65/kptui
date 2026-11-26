use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent};
use keepass::{Database, DatabaseKey};
use ratatui::{
	Frame, Terminal,
	backend::CrosstermBackend,
	style::Style,
	widgets::{Block, ListState, TableState},
};
use zeroize::{Zeroize, Zeroizing};

use crate::config::{Config, load_config};
use crate::db::Entry;
use crate::input::SECRET_CAP;
use crate::input::edit::{handle_edit_input, process_pending_save};
use crate::input::fieldedit::FieldEditor;
use crate::input::index::handle_index_input;
use crate::input::login::{attempt_unlock, create_database_with_password, database_missing, handle_login_input, request_unlock};
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
	pub next: Zeroizing<String>,
	pub expires: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KdfChange {
	Iterations(u64),
	Memory(u64),
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
	pub pending_save: Option<keepass::config::DatabaseVersion>,
	pub pending_unlock: bool,
	pub pending_kdf_change: Option<KdfChange>,
	pub confirm_delete: bool,
	pub confirm_exit: bool,
	/// Whether the password of the open entry is shown (Control-R), instead of dots.
	pub show_password: bool,
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

// Test convenience: invalid test configurations are failures, never defaults.
#[cfg(test)]
pub fn start(slim_mode: bool, password: Zeroizing<String>) -> App {
	try_start(slim_mode, password).expect("test configuration should load")
}

pub fn try_start(slim_mode: bool, password: Zeroizing<String>) -> anyhow::Result<App> {
	let mut config = load_config()?;
	let _ = crate::theme::ensure_default_themes();
	if !config.is_recovering() && config.default_database.is_none() {
		let default_path = crate::util::default_new_database_path();
		if default_path.is_file() {
			config.default_database = Some(default_path);
			let _ = crate::config::save_config(&config);
		}
	}
	let theme = load_theme(config.theme.as_deref()).unwrap_or_default();
	let mut app = App {
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
		pending_save: None,
		pending_unlock: false,
		pending_kdf_change: None,
		confirm_delete: false,
		confirm_exit: false,
		show_password: false,
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
	};
	app.db_version = app.config.default_database.as_deref().and_then(crate::db::kdbx_version);
	if app.config.is_recovering() {
		app.password.zeroize();
		crate::input::command::open_settings(&mut app);
		app.login_settings = true;
		app.status = app.config.recovery_message();
		return Ok(app);
	}
	if !app.password.is_empty() {
		if database_missing(&app) {
			create_database_with_password(&mut app);
		} else {
			request_unlock(&mut app);
		}
	}
	Ok(app)
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
}

pub fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, slim_mode: bool, password: Zeroizing<String>) -> io::Result<()> {
	let mut app = try_start(slim_mode, password).map_err(|error| io::Error::other(format!("{error:#}")))?;
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
		if app.pending_unlock {
			attempt_unlock(&mut app);
			continue;
		}
		if app.pending_kdf_change.is_some() {
			crate::input::settings::process_kdf_change(&mut app);
			continue;
		}
		if app.pending_save.is_some() {
			process_pending_save(&mut app);
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

#[cfg(test)]
mod startup_tests {
	use super::*;

	#[test]
	fn malformed_toml_never_gets_replaced_even_when_default_vault_exists() {
		let path = crate::util::expand_tilde("~/.config/kptui/config.toml");
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		let vault = crate::util::default_new_database_path();
		std::fs::create_dir_all(vault.parent().unwrap()).unwrap();
		std::fs::write(&vault, b"existing vault sentinel").unwrap();
		let text = "database = [\n";
		std::fs::write(&path, text).unwrap();
		assert!(try_start(false, Zeroizing::new(String::new())).is_err());
		assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
		assert_eq!(std::fs::read(&vault).unwrap(), b"existing vault sentinel");
	}

	#[test]
	fn invalid_values_open_configuration_and_esc_does_not_unlock() {
		let path = crate::util::expand_tilde("~/.config/kptui/config.toml");
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		let text = "auto_exit = -1\ndatabase = \"/keep/vault.kdbx\"\n";
		std::fs::write(&path, text).unwrap();
		let mut app = try_start(false, Zeroizing::new("unused password".to_string())).unwrap();
		assert!(app.screen == Screen::Settings && app.login_settings);
		assert!(app.password.is_empty());
		assert!(app.config.field_invalid("auto_exit"));
		assert!(crate::config::save_config(&app.config).is_err());
		handle_key(&mut app, crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Esc));
		assert!(app.should_quit);
		assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
	}
}
