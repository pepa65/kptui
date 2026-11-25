use std::path::Path;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use zeroize::Zeroize;

use crate::app::{App, ExportStep, ImportStep, KdfChange, KeyfileChangeStep, PasswordChangeStep, Screen};
use crate::config::save_config;
use crate::db::{Entry, build_database_key, save_database, unlock_database};
use crate::input::fieldedit::{EditAction, FieldEditor};
use crate::input::{secret_pop, secret_push};
use crate::theme::load_theme;
use crate::util::expand_tilde;

pub const UNLOCKED_ROW_COUNT: usize = 11;
pub const LOCKED_ROW_COUNT: usize = 7;

pub const DATABASE_ROW: usize = 0;
pub const KEYFILE_ROW: usize = 1;
pub const ITERATIONS_ROW: usize = 2;
pub const MEMORY_ROW: usize = 3;
pub const AUTO_EXIT_ROW: usize = 4;
pub const THEME_ROW: usize = 5;
pub const TMUX_CLEAR_ROW: usize = 6;
pub const FORMAT_ROW: usize = 7;
pub const CHANGE_PASSWORD_ROW: usize = 8;
pub const IMPORT_ROW: usize = 9;
pub const EXPORT_ROW: usize = 10;

pub(crate) fn settings_row(_app: &App, visible_row: usize) -> usize {
	visible_row
}

pub(crate) fn format_save_target(app: &App) -> keepass::config::DatabaseVersion {
	match app.kdbx.as_ref().map(|db| &db.config.version) {
		Some(keepass::config::DatabaseVersion::KDB4(1)) => keepass::config::DatabaseVersion::KDB4(0),
		_ => keepass::config::DatabaseVersion::KDB4(1),
	}
}

pub(crate) fn process_format_save(app: &mut App, version: keepass::config::DatabaseVersion) {
	let message = format!("Saved as {version}");
	let (Some(db), Some(key), Some(path)) = (app.kdbx.as_mut(), app.db_key.as_ref(), app.config.default_database.as_ref()) else {
		app.status = Some("Not saved: database is locked".to_string());
		return;
	};
	app.status = Some(match crate::db::save_database_as(path, key, db, &mut app.entries, version) {
		Ok(()) => message,
		Err(err) => format!("Failed to save: {err:#}"),
	});
}

fn visible_row_count(app: &App) -> usize {
	if app.login_settings { LOCKED_ROW_COUNT } else { UNLOCKED_ROW_COUNT }
}

pub fn handle_settings_input(app: &mut App, key: KeyEvent) {
	if app.pending_kdf_change.is_some() {
		return;
	}

	if app.exporting_database {
		handle_export_input(app, key);
		return;
	}

	if app.importing_database {
		handle_import_input(app, key);
		return;
	}

	if app.changing_keyfile {
		handle_change_keyfile_input(app, key.code);
		return;
	}

	if app.changing_password {
		handle_change_password_input(app, key.code);
		return;
	}

	if app.choosing_theme {
		handle_theme_picker_input(app, key.code);
		return;
	}

	if app.editing_field {
		handle_field_input(app, key);
		return;
	}

	match key.code {
		KeyCode::Down => {
			app.status = None;
			let count = visible_row_count(app);
			let selected = app.settings_state.selected().unwrap_or(0);
			let next = if selected + 1 >= count { 0 } else { selected + 1 };
			app.settings_state.select(Some(next));
		}

		KeyCode::Up => {
			app.status = None;
			let count = visible_row_count(app);
			let selected = app.settings_state.selected().unwrap_or(0);
			let prev = if selected == 0 { count - 1 } else { selected - 1 };
			app.settings_state.select(Some(prev));
		}

		KeyCode::Enter => activate_selected(app),

		KeyCode::Esc => {
			app.status = None;
			if app.login_settings {
				// Return to Login without exiting the application.
				app.screen = Screen::Login;
				app.login_settings = false;
				app.settings_state.select(Some(0));
			} else {
				app.screen = Screen::Index;
			}
		}

		_ => {}
	}
}

fn activate_selected(app: &mut App) {
	let selected = settings_row(app, app.settings_state.selected().unwrap_or(0));

	// Login settings must never expose authenticated database operations.
	if app.login_settings && matches!(selected, FORMAT_ROW | CHANGE_PASSWORD_ROW | IMPORT_ROW | EXPORT_ROW) {
		return;
	}

	match selected {
		DATABASE_ROW => start_editing_database(app),
		KEYFILE_ROW => {
			if app.login_settings {
				start_editing_keyfile_config(app);
			} else {
				start_change_keyfile(app);
			}
		}
		FORMAT_ROW if !app.login_settings => {
			let version = format_save_target(app);
			app.status = Some(format!("Saving as {version}..."));
			app.pending_save = Some(version);
		}
		ITERATIONS_ROW => start_editing_kdf_iterations(app),
		MEMORY_ROW => {
			app.field_editor = Some(FieldEditor::with_text((app.config.kdf_memory as f64 / crate::config::MIB as f64).to_string()));
			app.editing_field = true;
			app.status = None;
		}
		AUTO_EXIT_ROW => start_editing_auto_exit(app),
		THEME_ROW => start_choosing_theme(app),
		TMUX_CLEAR_ROW => start_editing_tmux_clear(app),
		CHANGE_PASSWORD_ROW => start_change_password(app),
		IMPORT_ROW => start_import(app),
		EXPORT_ROW => start_export(app),
		_ => {}
	}
}

fn start_editing_database(app: &mut App) {
	let value = app.config.default_database.as_ref().map(|p| p.display().to_string()).unwrap_or_default();

	app.field_editor = Some(FieldEditor::with_text(value));
	app.editing_field = true;
	app.status = None;
}

fn start_editing_keyfile_config(app: &mut App) {
	let value = app.config.keyfile.as_ref().map(|p| p.display().to_string()).unwrap_or_default();

	app.field_editor = Some(FieldEditor::with_text(value));
	app.editing_field = true;
	app.status = None;
}

fn start_change_keyfile(app: &mut App) {
	app.keyfile_path_buffer.clear();
	app.current_password_buffer.zeroize();
	app.keyfile_change_step = KeyfileChangeStep::Current;
	app.changing_keyfile = true;
	app.status = None;
}

fn cancel_change_keyfile(app: &mut App) {
	app.changing_keyfile = false;
	app.keyfile_change_step = KeyfileChangeStep::Current;
	app.current_password_buffer.zeroize();
	app.keyfile_path_buffer.clear();
	app.status = None;
}

fn handle_change_keyfile_input(app: &mut App, key: KeyCode) {
	match key {
		KeyCode::Char(c) => {
			app.status = None;

			match app.keyfile_change_step {
				KeyfileChangeStep::Current => {
					secret_push(&mut app.current_password_buffer, c);
				}
				KeyfileChangeStep::Path => {
					let limit = app.max_len.max(1);
					if c.len_utf8() <= limit.saturating_sub(app.keyfile_path_buffer.len()) {
						app.keyfile_path_buffer.push(c);
					}
				}
			}
		}

		KeyCode::Backspace => {
			app.status = None;
			match app.keyfile_change_step {
				KeyfileChangeStep::Current => {
					secret_pop(&mut app.current_password_buffer);
				}
				KeyfileChangeStep::Path => {
					app.keyfile_path_buffer.pop();
				}
			}
		}
		KeyCode::Enter => match app.keyfile_change_step {
			KeyfileChangeStep::Current => verify_keyfile_change_password(app),
			KeyfileChangeStep::Path => commit_keyfile_change(app),
		},
		KeyCode::Esc => cancel_change_keyfile(app),
		_ => {}
	}
}

fn verify_keyfile_change_password(app: &mut App) {
	let Some(path) = app.config.default_database.clone() else {
		app.status = Some("no database configured".to_string());
		cancel_change_keyfile(app);
		return;
	};

	match unlock_database(&path, &app.current_password_buffer, app.config.keyfile.as_deref()) {
		Ok((db, key, entries)) => {
			app.kdbx = Some(db);
			app.db_key = Some(key);
			app.entries = entries;
			app.keyfile_change_step = KeyfileChangeStep::Path;
			app.status = None;
		}
		Err(err) => {
			app.current_password_buffer.zeroize();
			app.status = Some(if app.config.keyfile.is_some() {
				format!("couldn't verify current password and keyfile: {err}")
			} else {
				"Invalid current master password".to_string()
			});
		}
	}
}

fn commit_keyfile_change(app: &mut App) {
	let raw = app.keyfile_path_buffer.trim();
	let new_keyfile = if raw.is_empty() { None } else { Some(expand_tilde(raw)) };

	if let Some(path) = new_keyfile.as_ref()
		&& !path.is_file()
	{
		app.status = Some(format!("no keyfile found at {}", path.display()));
		return;
	}

	let password = std::mem::take(&mut app.current_password_buffer);
	let result = rotate_database_credentials(app, &password, new_keyfile.as_deref());
	app.current_password_buffer = password;
	match result {
		Ok(()) => {
			app.config.keyfile = new_keyfile;
			app.status = Some(match save_config(&app.config) {
				Ok(()) => "keyfile changed".to_string(),
				Err(err) => format!("database credentials changed, but couldn't save config: {err}"),
			});
			app.changing_keyfile = false;
			app.keyfile_path_buffer.clear();
			app.current_password_buffer.zeroize();
		}
		Err(err) => {
			app.status = Some(format!("couldn't change keyfile: {err:#}"));
		}
	}
}

fn start_editing_kdf_iterations(app: &mut App) {
	app.field_editor = Some(FieldEditor::with_text(app.config.kdf_iterations.to_string()));
	app.editing_field = true;
	app.status = None;
}

fn change_kdf_iterations(app: &mut App, new_iterations: u64) -> anyhow::Result<()> {
	let path = app.config.default_database.clone().ok_or_else(|| anyhow::anyhow!("no database configured"))?;
	let key = app.db_key.clone().ok_or_else(|| anyhow::anyhow!("no unlocked database"))?;
	let db = app.kdbx.as_mut().ok_or_else(|| anyhow::anyhow!("no unlocked database"))?;
	crate::db::reconcile_kdf_iterations(&path, &key, db, &mut app.entries, new_iterations)?;
	Ok(())
}

pub(crate) fn process_kdf_change(app: &mut App) {
	let Some(change) = app.pending_kdf_change.take() else { return };
	let description = match change {
		KdfChange::Iterations(new) => format!("KDF iterations {} -> {new}", app.config.kdf_iterations),
		KdfChange::Memory(new) => format!("KDF memory {} -> {} MiB", app.config.kdf_memory as f64 / crate::config::MIB as f64, new / crate::config::MIB),
	};
	let result = match change {
		KdfChange::Iterations(value) => change_kdf_iterations(app, value),
		KdfChange::Memory(value) => change_kdf_memory(app, value),
	};
	if let Err(err) = result {
		app.status = Some(format!("Failed to save {description}: {err:#}"));
		return;
	}
	match change {
		KdfChange::Iterations(value) => app.config.kdf_iterations = value,
		KdfChange::Memory(value) => app.config.kdf_memory = value,
	}
	app.status = Some(match save_config(&app.config) {
		Ok(()) => format!("Saved {description}"),
		Err(err) => format!("Saved {description}, but couldn't save configuration: {err:#}"),
	});
	app.last_activity = std::time::Instant::now();
}

fn change_kdf_memory(app: &mut App, memory: u64) -> anyhow::Result<()> {
	let path = app.config.default_database.as_deref().ok_or_else(|| anyhow::anyhow!("no database configured"))?;
	let key = app.db_key.as_ref().ok_or_else(|| anyhow::anyhow!("no unlocked database"))?;
	let db = app.kdbx.as_mut().ok_or_else(|| anyhow::anyhow!("no unlocked database"))?;
	let previous_kdf = db.config.kdf_config.clone();
	match &mut db.config.kdf_config {
		keepass::config::KdfConfig::Argon2 { memory: current, .. } | keepass::config::KdfConfig::Argon2id { memory: current, .. } => *current = memory,
		_ => anyhow::bail!("database does not use Argon2"),
	}
	if let Err(err) = save_database(path, key, db, &mut app.entries) {
		db.config.kdf_config = previous_kdf;
		return Err(err);
	}
	Ok(())
}

fn start_editing_auto_exit(app: &mut App) {
	app.field_editor = Some(FieldEditor::with_text(app.config.auto_exit.as_secs().to_string()));
	app.editing_field = true;
	app.status = None;
}

fn start_editing_tmux_clear(app: &mut App) {
	app.field_editor = Some(FieldEditor::with_text(app.config.tmux_clear.as_secs().to_string()));
	app.editing_field = true;
	app.status = None;
}

fn handle_field_input(app: &mut App, key: KeyEvent) {
	let Some(editor) = app.field_editor.as_mut() else {
		app.editing_field = false;
		return;
	};

	match editor.handle_key(key) {
		EditAction::Continue => {}
		EditAction::Accept => {
			commit_field(app);
		}
		EditAction::Cancel => {
			app.editing_field = false;
			app.field_editor = None;
			app.status = None;
		}
	}
}

fn commit_field(app: &mut App) {
	let selected = settings_row(app, app.settings_state.selected().unwrap_or(0));
	let value = app.field_editor.take().map(FieldEditor::into_text).unwrap_or_default();
	app.editing_field = false;
	match selected {
		DATABASE_ROW => {
			app.config.default_database = if value.trim().is_empty() { None } else { Some(expand_tilde(value.trim())) };
			app.db_version = app.config.default_database.as_deref().and_then(crate::db::kdbx_version);
		}

		KEYFILE_ROW => {
			let raw = value.trim();
			let keyfile = if raw.is_empty() {
				None
			} else {
				let path = expand_tilde(raw);
				if !path.is_file() {
					app.status = Some(format!("no keyfile found at {}", path.display()));
					return;
				}

				Some(path)
			};
			app.config.keyfile = keyfile;
			app.status = Some(match save_config(&app.config) {
				Ok(()) => "keyfile configuration saved".to_string(),
				Err(err) => format!("couldn't save config: {err}"),
			});
			return;
		}

		ITERATIONS_ROW => {
			let iterations = match parse_kdf_iterations(&value) {
				Ok(iterations) => iterations,
				Err(err) => {
					app.status = Some(err);
					return;
				}
			};

			if iterations != app.config.kdf_iterations {
				if app.kdbx.is_some() {
					app.status = Some(format!("Saving KDF iterations {} -> {iterations}...", app.config.kdf_iterations));
					app.pending_kdf_change = Some(KdfChange::Iterations(iterations));
					return;
				}

				app.config.kdf_iterations = iterations;
			}
		}

		MEMORY_ROW => {
			let memory = match parse_kdf_memory(&value) {
				Ok(memory) => memory,
				Err(err) => {
					app.status = Some(err);
					return;
				}
			};
			if memory != app.config.kdf_memory {
				if app.kdbx.is_some() {
					let old = app.config.kdf_memory as f64 / crate::config::MIB as f64;
					let new = memory / crate::config::MIB;
					app.status = Some(format!("Saving KDF memory {old} -> {new} MiB..."));
					app.pending_kdf_change = Some(KdfChange::Memory(memory));
					return;
				}
				app.config.kdf_memory = memory;
			}
		}

		AUTO_EXIT_ROW => match parse_seconds(&value) {
			Ok(secs) => app.config.auto_exit = Duration::from_secs(secs),
			Err(err) => {
				app.status = Some(err);
				return;
			}
		},

		TMUX_CLEAR_ROW => match parse_seconds(&value) {
			Ok(secs) => app.config.tmux_clear = Duration::from_secs(secs),
			Err(err) => {
				app.status = Some(err);
				return;
			}
		},

		_ => return,
	}

	app.status = Some(match save_config(&app.config) {
		Ok(()) => "saved".to_string(),
		Err(err) => format!("couldn't save config: {err}"),
	});
}

fn parse_kdf_iterations(value: &str) -> Result<u64, String> {
	let iterations = value.trim().parse::<u64>().map_err(|_| format!("\"{}\" isn't a whole number", value.trim()))?;

	if iterations == 0 {
		return Err("KDF iterations must be greater than zero".to_string());
	}

	Ok(iterations)
}

fn parse_kdf_memory(value: &str) -> Result<u64, String> {
	let mib = value.trim().parse::<u64>().map_err(|_| "KDF memory must be a positive whole number of MiB".to_string())?;
	if mib == 0 {
		return Err("KDF memory must be at least 1 MiB".to_string());
	}
	if mib > u64::from(u32::MAX) / 1024 {
		return Err("KDF memory is too large".to_string());
	}
	Ok(mib * crate::config::MIB)
}

fn parse_seconds(value: &str) -> Result<u64, String> {
	value.trim().parse::<u64>().map_err(|_| format!("\"{}\" isn't a whole number of seconds", value.trim()))
}

fn start_change_password(app: &mut App) {
	app.current_password_buffer.zeroize();
	app.new_password_buffer.zeroize();
	app.new_password_confirm.zeroize();
	app.password_change_step = PasswordChangeStep::Current;
	app.changing_password = true;
	app.status = None;
}

fn cancel_change_password(app: &mut App) {
	app.changing_password = false;
	app.password_change_step = PasswordChangeStep::Current;
	app.current_password_buffer.zeroize();
	app.new_password_buffer.zeroize();
	app.new_password_confirm.zeroize();
	app.status = None;
}

fn handle_change_password_input(app: &mut App, key: KeyCode) {
	match key {
		KeyCode::Char(c) => {
			app.status = None;

			let buffer = match app.password_change_step {
				PasswordChangeStep::Current => &mut app.current_password_buffer,
				PasswordChangeStep::New => &mut app.new_password_buffer,
				PasswordChangeStep::ConfirmNew => &mut app.new_password_confirm,
			};

			secret_push(buffer, c);
		}

		KeyCode::Backspace => {
			app.status = None;

			let buffer = match app.password_change_step {
				PasswordChangeStep::Current => &mut app.current_password_buffer,
				PasswordChangeStep::New => &mut app.new_password_buffer,
				PasswordChangeStep::ConfirmNew => &mut app.new_password_confirm,
			};

			secret_pop(buffer);
		}

		KeyCode::Enter => match app.password_change_step {
			PasswordChangeStep::Current => verify_current_password(app),
			PasswordChangeStep::New => {
				if app.new_password_buffer.is_empty() {
					app.status = Some("new password can't be empty".to_string());
				} else {
					app.password_change_step = PasswordChangeStep::ConfirmNew;
				}
			}
			PasswordChangeStep::ConfirmNew => commit_password_change(app),
		},

		KeyCode::Esc => cancel_change_password(app),

		_ => {}
	}
}

fn verify_current_password(app: &mut App) {
	let Some(path) = app.config.default_database.clone() else {
		app.status = Some("no unlocked database".to_string());
		cancel_change_password(app);
		return;
	};

	match unlock_database(&path, &app.current_password_buffer, app.config.keyfile.as_deref()) {
		Ok((db, key, entries)) => {
			app.kdbx = Some(db);
			app.db_key = Some(key);
			app.entries = entries;

			app.current_password_buffer.zeroize();
			app.status = None;
			app.password_change_step = PasswordChangeStep::New;
		}
		Err(err) => {
			app.status = Some(if app.config.keyfile.is_some() {
				format!("couldn't verify current password and keyfile: {err}")
			} else {
				"Invalid current master password".to_string()
			});
			app.current_password_buffer.zeroize();
		}
	}
}

fn rotate_database_credentials(app: &mut App, new_password: &zeroize::Zeroizing<String>, new_keyfile: Option<&Path>) -> anyhow::Result<()> {
	let path = app.config.default_database.clone().ok_or_else(|| anyhow::anyhow!("no database configured"))?;
	let new_key = build_database_key(new_password, new_keyfile)?;
	let db = app.kdbx.as_mut().ok_or_else(|| anyhow::anyhow!("no unlocked database"))?;
	save_database(&path, &new_key, db, &mut app.entries)?;
	app.db_key = Some(new_key);
	Ok(())
}

fn commit_password_change(app: &mut App) {
	if app.new_password_confirm != app.new_password_buffer {
		app.status = Some("passwords don't match".to_string());
		app.new_password_confirm.zeroize();
		app.password_change_step = PasswordChangeStep::New;
		return;
	}

	let new_password = std::mem::replace(&mut app.new_password_buffer, zeroize::Zeroizing::new(String::with_capacity(crate::input::SECRET_CAP)));
	let keyfile = app.config.keyfile.clone();
	let result = rotate_database_credentials(app, &new_password, keyfile.as_deref());
	app.status = Some(match result {
		Ok(()) => "master password changed".to_string(),
		Err(err) => format!("couldn't change password: {err:#}"),
	});

	app.changing_password = false;
	app.password_change_step = PasswordChangeStep::Current;
	app.current_password_buffer.zeroize();
	app.new_password_buffer.zeroize();
	app.new_password_confirm.zeroize();
}

fn start_import(app: &mut App) {
	reset_import_state(app);
	app.importing_database = true;
	app.field_editor = Some(FieldEditor::default());
	app.editing_field = true;
	app.status = None;
}

fn reset_import_state(app: &mut App) {
	app.importing_database = false;
	app.import_step = ImportStep::Path;
	app.field_editor = None;
	app.editing_field = false;
	app.import_kdbx_password_buffer.zeroize();
	app.pending_import_path = None;
}

fn cancel_import(app: &mut App) {
	reset_import_state(app);
	app.status = None;
}

fn handle_path_input(app: &mut App, key: KeyEvent, confirm: fn(&mut App), cancel: fn(&mut App)) {
	let Some(editor) = app.field_editor.as_mut() else { return };
	match editor.handle_key(key) {
		EditAction::Continue => app.status = None,
		EditAction::Accept => confirm(app),
		EditAction::Cancel => cancel(app),
	}
}

fn handle_import_input(app: &mut App, key: KeyEvent) {
	if app.import_step == ImportStep::Path {
		handle_path_input(app, key, confirm_import_path, cancel_import);
		return;
	}

	match key.code {
		KeyCode::Char(c) => {
			app.status = None;
			secret_push(&mut app.import_kdbx_password_buffer, c);
		}
		KeyCode::Backspace => {
			app.status = None;
			secret_pop(&mut app.import_kdbx_password_buffer);
		}
		KeyCode::Enter => confirm_import_kdbx_password(app),
		KeyCode::Esc => cancel_import(app),
		_ => {}
	}
}

fn confirm_import_path(app: &mut App) {
	let raw = app.field_editor.as_ref().map_or("", |editor| editor.text().trim());

	if raw.is_empty() {
		app.status = Some("enter a file path".to_string());
		return;
	}

	let path = expand_tilde(raw);

	if !path.is_file() {
		app.status = Some(format!("no file found at {}", path.display()));
		return;
	}

	match crate::import::detect_format(&path) {
		Ok(crate::import::ImportFormat::Kdbx) => {
			app.pending_import_path = Some(path);
			app.import_kdbx_password_buffer.zeroize();
			app.import_step = ImportStep::KdbxPassword;
			app.field_editor = None;
			app.editing_field = false;
			app.status = None;
		}

		Ok(crate::import::ImportFormat::Csv) => match crate::import::import_csv(&path) {
			Ok(entries) => finish_import(app, entries, &path),
			Err(err) => app.status = Some(format!("couldn't import: {err:#}")),
		},

		Ok(crate::import::ImportFormat::Json) => match crate::import::import_json(&path) {
			Ok(entries) => finish_import(app, entries, &path),
			Err(err) => app.status = Some(format!("couldn't import: {err:#}")),
		},

		Err(err) => app.status = Some(format!("{err:#}")),
	}
}

fn confirm_import_kdbx_password(app: &mut App) {
	let Some(path) = app.pending_import_path.clone() else {
		cancel_import(app);
		return;
	};

	match crate::import::import_kdbx(&path, &app.import_kdbx_password_buffer) {
		Ok(entries) => finish_import(app, entries, &path),
		Err(err) => {
			app.status = Some(format!("{err:#}"));
			app.import_kdbx_password_buffer.zeroize();
		}
	}
}

fn finish_import(app: &mut App, imported: Vec<Entry>, source: &Path) {
	let count = imported.len();

	for mut entry in imported {
		entry.id = None;
		app.entries.push(entry);
	}

	app.refresh_filter();

	let (Some(path), Some(key), Some(db)) = (app.config.default_database.clone(), app.db_key.clone(), app.kdbx.as_mut()) else {
		app.status = Some(format!("imported {count} entries from {}, but couldn't save: database is locked", source.display()));
		reset_import_state(app);
		return;
	};

	app.status = Some(match save_database(&path, &key, db, &mut app.entries) {
		Ok(()) => format!("imported {count} entries from {}", source.display()),
		Err(err) => format!("imported {count} entries, but failed to save: {err:#}"),
	});

	reset_import_state(app);
}

fn start_export(app: &mut App) {
	reset_export_state(app);
	app.exporting_database = true;
	app.field_editor = Some(FieldEditor::default());
	app.editing_field = true;
	app.status = None;
}

fn reset_export_state(app: &mut App) {
	app.exporting_database = false;
	app.export_step = ExportStep::Path;
	app.field_editor = None;
	app.editing_field = false;
	app.pending_export_path = None;
}

fn cancel_export(app: &mut App) {
	reset_export_state(app);
	app.status = None;
}

fn handle_export_input(app: &mut App, key: KeyEvent) {
	if app.export_step == ExportStep::Path {
		handle_path_input(app, key, confirm_export_path, cancel_export);
		return;
	}

	match key.code {
		KeyCode::Enter => do_export(app),
		KeyCode::Esc => cancel_export(app),
		_ => {}
	}
}

fn confirm_export_path(app: &mut App) {
	let raw = app.field_editor.as_ref().map_or("", |editor| editor.text().trim());

	if raw.is_empty() {
		app.status = Some("enter a file path".to_string());
		return;
	}

	let path = expand_tilde(raw);

	let format = match crate::export::detect_format(&path) {
		Ok(format) => format,
		Err(err) => {
			app.status = Some(format!("{err:#}"));
			return;
		}
	};

	app.pending_export_path = Some(path.clone());

	let plaintext = matches!(format, crate::export::ExportFormat::Csv | crate::export::ExportFormat::Json);
	let overwrite = path.exists();

	if plaintext || overwrite {
		app.export_step = ExportStep::Confirm;
		app.field_editor = None;
		app.editing_field = false;
		app.status = None;
	} else {
		do_export(app);
	}
}

fn do_export(app: &mut App) {
	let Some(path) = app.pending_export_path.clone() else {
		cancel_export(app);
		return;
	};

	let format = match crate::export::detect_format(&path) {
		Ok(format) => format,
		Err(err) => {
			app.status = Some(format!("{err:#}"));
			reset_export_state(app);
			return;
		}
	};

	let count = app.entries.len();

	let result = match format {
		crate::export::ExportFormat::Kdbx => {
			let (Some(key), Some(db)) = (app.db_key.clone(), app.kdbx.as_mut()) else {
				app.status = Some("no unlocked database".to_string());
				reset_export_state(app);
				return;
			};

			save_database(&path, &key, db, &mut app.entries)
		}
		crate::export::ExportFormat::Csv => crate::export::export_csv(&path, &app.entries),
		crate::export::ExportFormat::Json => crate::export::export_json(&path, &app.entries),
	};

	app.status = Some(match result {
		Ok(()) => format!("exported {count} entries to {}", path.display()),
		Err(err) => format!("couldn't export: {err:#}"),
	});

	reset_export_state(app);
}

fn start_choosing_theme(app: &mut App) {
	if app.available_themes.is_empty() {
		app.status = Some("no themes found in ~/.config/kptui/themes".to_string());
		return;
	}

	let current = app.config.theme.as_deref().map(crate::theme::slugify);
	let start = app
		.available_themes
		.iter()
		.position(|name| current.as_deref().is_some_and(|c| c == crate::theme::slugify(name)))
		.unwrap_or(0);

	app.theme_state.select(Some(start));
	app.choosing_theme = true;
	app.status = None;
}

fn handle_theme_picker_input(app: &mut App, key: KeyCode) {
	let count = app.available_themes.len();

	match key {
		KeyCode::Down => {
			if count == 0 {
				return;
			}
			let selected = app.theme_state.selected().unwrap_or(0);
			let next = if selected + 1 >= count { 0 } else { selected + 1 };
			app.theme_state.select(Some(next));
		}

		KeyCode::Up => {
			if count == 0 {
				return;
			}
			let selected = app.theme_state.selected().unwrap_or(0);
			let prev = if selected == 0 { count - 1 } else { selected - 1 };
			app.theme_state.select(Some(prev));
		}

		KeyCode::Enter => {
			apply_selected_theme(app);
			app.choosing_theme = false;
		}

		KeyCode::Esc => {
			app.choosing_theme = false;
			app.status = None;
		}

		_ => {}
	}
}

fn apply_selected_theme(app: &mut App) {
	let Some(selected) = app.theme_state.selected() else {
		return;
	};
	let Some(name) = app.available_themes.get(selected).cloned() else {
		return;
	};

	match load_theme(Some(&name)) {
		Ok(theme) => {
			app.theme = theme;
			app.config.theme = Some(crate::theme::slugify(&name));

			app.status = Some(match save_config(&app.config) {
				Ok(()) => format!("theme set to {name}"),
				Err(err) => format!("theme applied, but couldn't save config: {err}"),
			});
		}

		Err(err) => {
			app.status = Some(format!("couldn't load theme \"{name}\": {err}"));
		}
	}
}

#[test]
fn settings_rows_are_contiguous() {
	assert_eq!(DATABASE_ROW, 0);
	assert_eq!(KEYFILE_ROW, 1);
	assert_eq!(TMUX_CLEAR_ROW, 6);
	assert_eq!(FORMAT_ROW, 7);
	assert_eq!(ITERATIONS_ROW, 2);
	assert_eq!(MEMORY_ROW, 3);
	assert_eq!(AUTO_EXIT_ROW, 4);
	assert_eq!(THEME_ROW, 5);
	assert_eq!(CHANGE_PASSWORD_ROW, 8);
	assert_eq!(IMPORT_ROW, 9);
	assert_eq!(EXPORT_ROW, 10);
	assert_eq!(UNLOCKED_ROW_COUNT, 11);
	assert_eq!(LOCKED_ROW_COUNT, 7);
}

#[cfg(test)]
mod format_selection_tests {
	use super::*;
	use keepass::config::DatabaseVersion;
	use ratatui::{Terminal, backend::TestBackend};
	use zeroize::Zeroizing;

	#[test]
	fn configuration_format_action_saves_and_updates_its_label() {
		let path = std::env::temp_dir().join(format!("kptui-config-format-{}.kdbx", std::process::id()));
		let password = Zeroizing::new("format-test".to_string());
		let (db, key, entries) = crate::db::create_database(&path, 1, &password, None, crate::config::MIB).unwrap();
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		app.screen = Screen::Settings;
		app.db_version = Some("KDBX4.1".to_string());
		app.kdbx = Some(db);
		app.db_key = Some(key);
		app.entries = entries;
		app.config.default_database = Some(path.clone());
		app.settings_state.select(Some(FORMAT_ROW));
		let mut terminal = Terminal::new(TestBackend::new(96, 26)).unwrap();

		for minor in [0, 1] {
			let frame = terminal
				.draw(|frame| {
					crate::app::draw(frame, &mut app);
				})
				.unwrap();
			let rendered: String = frame.buffer.content.iter().map(|cell| cell.symbol()).collect();
			assert!(rendered.contains(&format!("Save as KDBX4.{minor}")));
			assert!(rendered.contains(&format!("Configuration 'config' KDBX4.{}", 1 - minor)));
			handle_settings_input(&mut app, KeyEvent::from(KeyCode::Enter));
			assert_eq!(app.pending_save, Some(DatabaseVersion::KDB4(minor)));
			crate::input::edit::process_pending_save(&mut app);
			assert!(app.screen == Screen::Settings);
			assert_eq!(app.status, Some(format!("Saved as KDBX4.{minor}")));
			let (db, _, _) = crate::db::unlock_database(&path, &password, None).unwrap();
			assert_eq!(db.config.version, DatabaseVersion::KDB4(minor));
			assert_eq!(format_save_target(&app), DatabaseVersion::KDB4(1 - minor));
		}

		// A failed conversion keeps the current database format.
		app.config.default_database = Some(path.join("missing.kdbx"));
		handle_settings_input(&mut app, KeyEvent::from(KeyCode::Enter));
		crate::input::edit::process_pending_save(&mut app);
		assert!(app.status.as_deref().unwrap().starts_with("Failed to save:"));
		assert_eq!(app.kdbx.as_ref().unwrap().config.version, DatabaseVersion::KDB4(1));
		std::fs::remove_file(path).unwrap();
	}

	#[test]
	fn login_configuration_skips_format_selection_and_keeps_field_mapping() {
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		app.screen = Screen::Settings;
		app.login_settings = true;
		for row in 0..LOCKED_ROW_COUNT {
			assert_eq!(
				settings_row(&app, row),
				[DATABASE_ROW, KEYFILE_ROW, ITERATIONS_ROW, MEMORY_ROW, AUTO_EXIT_ROW, THEME_ROW, TMUX_CLEAR_ROW][row]
			);
		}
		app.settings_state.select(Some(2));
		handle_settings_input(&mut app, KeyEvent::from(KeyCode::Enter));
		assert_eq!(app.field_editor.as_ref().unwrap().text(), app.config.kdf_iterations.to_string());
		handle_settings_input(&mut app, KeyEvent::from(KeyCode::Esc));
		let mut terminal = Terminal::new(TestBackend::new(96, 26)).unwrap();
		let frame = terminal
			.draw(|frame| {
				crate::app::draw(frame, &mut app);
			})
			.unwrap();
		let rendered: String = frame.buffer.content.iter().map(|cell| cell.symbol()).collect();
		assert!(!rendered.contains("Save as KDBX"));
		assert!(rendered.contains("KDF iterations (time cost)"));
		assert!(!rendered.contains("Change master password"));
		app.settings_state.select(Some(4));
		handle_settings_input(&mut app, KeyEvent::from(KeyCode::Enter));
		assert_eq!(app.field_editor.as_ref().unwrap().text(), app.config.auto_exit.as_secs().to_string());
		handle_settings_input(&mut app, KeyEvent::from(KeyCode::Esc));
	}
}

#[cfg(test)]
mod memory_setting_tests {
	use super::*;
	use keepass::config::{DatabaseVersion, KdfConfig};
	use zeroize::Zeroizing;

	#[test]
	fn iterations_commit_announces_the_change_before_saving() {
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		app.screen = Screen::Settings;
		app.kdbx = Some(keepass::Database::new());
		app.config.kdf_iterations = 900;
		app.settings_state.select(Some(ITERATIONS_ROW));
		app.field_editor = Some(FieldEditor::with_text("90"));
		app.editing_field = true;
		handle_settings_input(&mut app, KeyEvent::from(KeyCode::Enter));
		assert_eq!(app.pending_kdf_change, Some(KdfChange::Iterations(90)));
		assert_eq!(app.config.kdf_iterations, 900);
		assert_eq!(app.status.as_deref(), Some("Saving KDF iterations 900 -> 90..."));
		let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(96, 26)).unwrap();
		let frame = terminal
			.draw(|frame| {
				crate::app::draw(frame, &mut app);
			})
			.unwrap();
		let rendered: String = frame.buffer.content.iter().map(|cell| cell.symbol()).collect();
		assert!(rendered.contains("Saving KDF iterations 900 -> 90..."));
	}

	#[test]
	fn memory_commit_announces_the_change_before_saving() {
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		app.screen = Screen::Settings;
		app.kdbx = Some(keepass::Database::new());
		app.config.kdf_memory = crate::config::MIB;
		app.settings_state.select(Some(MEMORY_ROW));
		app.field_editor = Some(FieldEditor::with_text("2"));
		app.editing_field = true;
		handle_settings_input(&mut app, KeyEvent::from(KeyCode::Enter));
		assert_eq!(app.pending_kdf_change, Some(KdfChange::Memory(2 * crate::config::MIB)));
		assert_eq!(app.config.kdf_memory, crate::config::MIB);
		assert_eq!(app.status.as_deref(), Some("Saving KDF memory 1 -> 2 MiB..."));
		let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(96, 26)).unwrap();
		let frame = terminal
			.draw(|frame| {
				crate::app::draw(frame, &mut app);
			})
			.unwrap();
		let rendered: String = frame.buffer.content.iter().map(|cell| cell.symbol()).collect();
		assert!(rendered.contains("Saving KDF memory 1 -> 2 MiB..."));
	}

	#[test]
	fn memory_input_requires_positive_whole_mib() {
		assert_eq!(parse_kdf_memory("256").unwrap(), 268_435_456);
		assert_eq!(parse_kdf_memory("1").unwrap(), crate::config::MIB);
		for text in ["0", "-1", "0.25", "1.5", "", "4194304", "18446744073709551615"] {
			assert!(parse_kdf_memory(text).is_err(), "{text}");
		}
	}

	#[test]
	fn memory_change_preserves_format_and_rolls_back_on_failure() {
		let path = std::env::temp_dir().join(format!("kptui-memory-setting-{}.kdbx", std::process::id()));
		let password = Zeroizing::new("memory-test".to_string());
		let (mut db, key, mut entries) = crate::db::create_database(&path, 1, &password, None, crate::config::MIB).unwrap();
		crate::db::save_database_as(&path, &key, &mut db, &mut entries, DatabaseVersion::KDB4(0)).unwrap();
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		app.kdbx = Some(db);
		app.db_key = Some(key);
		app.entries = entries;
		app.config.default_database = Some(path.clone());
		change_kdf_memory(&mut app, 2 * crate::config::MIB).unwrap();
		let (reopened, _, _) = crate::db::unlock_database(&path, &password, None).unwrap();
		assert_eq!(reopened.config.version, DatabaseVersion::KDB4(0));
		match reopened.config.kdf_config {
			KdfConfig::Argon2 { memory, iterations, .. } => {
				assert_eq!(memory, 2 * crate::config::MIB);
				assert_eq!(iterations, 1);
			}
			_ => panic!("expected Argon2"),
		}
		app.config.default_database = Some(path.join("missing.kdbx"));
		assert!(change_kdf_memory(&mut app, 3 * crate::config::MIB).is_err());
		match app.kdbx.as_ref().unwrap().config.kdf_config {
			KdfConfig::Argon2 { memory, .. } => assert_eq!(memory, 2 * crate::config::MIB),
			_ => panic!("expected Argon2"),
		}
		std::fs::remove_file(path).unwrap();
	}
}
