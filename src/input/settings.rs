use std::path::Path;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use zeroize::Zeroize;

use crate::app::{App, ExportStep, ImportStep, KeyfileChangeStep, PasswordChangeStep, Screen};
use crate::config::save_config;
use crate::db::{Entry, build_database_key, save_database, unlock_database};
use crate::input::fieldedit::{EditAction, FieldEditor};
use crate::input::{secret_pop, secret_push};
use crate::theme::load_theme;
use crate::util::expand_tilde;

pub const ROW_COUNT: usize = 8;
pub const CONFIG_ROW_COUNT: usize = 5;

pub const DATABASE_ROW: usize = 0;
pub const KEYFILE_ROW: usize = 1;
pub const ITERATIONS_ROW: usize = 2;
pub const AUTO_EXIT_ROW: usize = 3;
pub const THEME_ROW: usize = 4;
pub const CHANGE_PASSWORD_ROW: usize = 5;
pub const IMPORT_ROW: usize = 6;
pub const EXPORT_ROW: usize = 7;

fn visible_row_count(app: &App) -> usize {
	if app.login_settings { CONFIG_ROW_COUNT } else { ROW_COUNT }
}

pub fn handle_settings_input(app: &mut App, key: KeyEvent) {
	if app.exporting_database {
		handle_export_input(app, key);
		return;
	}

	if app.importing_database {
		handle_import_input(app, key.code);
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
	let selected = app.settings_state.selected().unwrap_or(0);

	// Login settings must never expose authenticated database operations.
	if app.login_settings && selected >= CHANGE_PASSWORD_ROW {
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
		ITERATIONS_ROW => start_editing_kdf_iterations(app),
		AUTO_EXIT_ROW => start_editing_auto_exit(app),
		THEME_ROW => start_choosing_theme(app),
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
			let limit = app.max_len.max(1);

			match app.keyfile_change_step {
				KeyfileChangeStep::Current => {
					if app.current_password_buffer.len() < limit {
						secret_push(&mut app.current_password_buffer, c);
					}
				}
				KeyfileChangeStep::Path => {
					if app.keyfile_path_buffer.len() < limit {
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
					secret_pop(&mut app.keyfile_path_buffer);
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

	let password = zeroize::Zeroizing::new(app.current_password_buffer.to_string());

	let result = rotate_database_credentials(app, &password, new_keyfile.as_deref());

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

	if let keepass::config::KdfConfig::Argon2 { ref mut iterations, .. } = db.config.kdf_config {
		*iterations = new_iterations;
	} else {
		anyhow::bail!("database does not use Argon2");
	}
	save_database(&path, &key, db, &mut app.entries)?;
	Ok(())
}

fn start_editing_auto_exit(app: &mut App) {
	app.field_editor = Some(FieldEditor::with_text(app.config.auto_exit.as_secs().to_string()));
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
	let selected = app.settings_state.selected().unwrap_or(0);
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
					if let Err(err) = change_kdf_iterations(app, iterations) {
						app.status = Some(format!("couldn't change KDF iterations: {err:#}"));
						return;
					}
				}

				app.config.kdf_iterations = iterations;
			}
		}

		AUTO_EXIT_ROW => match parse_seconds(&value) {
			Ok(secs) => app.config.auto_exit = Duration::from_secs(secs),
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
			let limit = app.max_len.max(1);

			let buffer = match app.password_change_step {
				PasswordChangeStep::Current => &mut app.current_password_buffer,
				PasswordChangeStep::New => &mut app.new_password_buffer,
				PasswordChangeStep::ConfirmNew => &mut app.new_password_confirm,
			};

			if buffer.len() < limit {
				secret_push(buffer, c);
			}
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

	let new_password = zeroize::Zeroizing::new(app.new_password_buffer.clone());
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
	app.status = None;
}

fn reset_import_state(app: &mut App) {
	app.importing_database = false;
	app.import_step = ImportStep::Path;
	app.import_path_buffer.clear();
	app.import_kdbx_password_buffer.zeroize();
	app.pending_import_path = None;
}

fn cancel_import(app: &mut App) {
	reset_import_state(app);
	app.status = None;
}

fn handle_import_input(app: &mut App, key: KeyCode) {
	match key {
		KeyCode::Char(c) => {
			app.status = None;
			let limit = app.max_len.max(1);

			let buffer = match app.import_step {
				ImportStep::Path => &mut app.import_path_buffer,
				ImportStep::KdbxPassword => &mut app.import_kdbx_password_buffer,
			};

			if buffer.len() < limit {
				secret_push(buffer, c);
			}
		}

		KeyCode::Backspace => {
			app.status = None;

			let buffer = match app.import_step {
				ImportStep::Path => &mut app.import_path_buffer,
				ImportStep::KdbxPassword => &mut app.import_kdbx_password_buffer,
			};

			secret_pop(buffer);
		}

		KeyCode::Enter => match app.import_step {
			ImportStep::Path => confirm_import_path(app),
			ImportStep::KdbxPassword => confirm_import_kdbx_password(app),
		},

		KeyCode::Esc => cancel_import(app),

		_ => {}
	}
}

fn confirm_import_path(app: &mut App) {
	let raw = app.import_path_buffer.trim();

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
	app.status = None;
}

fn reset_export_state(app: &mut App) {
	app.exporting_database = false;
	app.export_step = ExportStep::Path;
	app.export_path_buffer.clear();
	app.pending_export_path = None;
}

fn cancel_export(app: &mut App) {
	reset_export_state(app);
	app.status = None;
}

fn handle_export_input(app: &mut App, key: KeyEvent) {
	match key.code {
		KeyCode::Char(c) => {
			app.status = None;

			if app.export_step == ExportStep::Path {
				let limit = app.max_len.max(1);
				if app.export_path_buffer.len() < limit {
					app.export_path_buffer.push(c);
				}
			}
		}

		KeyCode::Backspace => {
			app.status = None;

			if app.export_step == ExportStep::Path {
				secret_pop(&mut app.export_path_buffer);
			}
		}

		KeyCode::Enter => match app.export_step {
			ExportStep::Path => confirm_export_path(app),
			ExportStep::Confirm => do_export(app),
		},

		KeyCode::Esc => cancel_export(app),

		_ => {}
	}
}

fn confirm_export_path(app: &mut App) {
	let raw = app.export_path_buffer.trim();

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
	assert_eq!(ITERATIONS_ROW, 2);
	assert_eq!(AUTO_EXIT_ROW, 3);
	assert_eq!(THEME_ROW, 4);
	assert_eq!(CHANGE_PASSWORD_ROW, 5);
	assert_eq!(IMPORT_ROW, 6);
	assert_eq!(EXPORT_ROW, 7);
	assert_eq!(ROW_COUNT, 8);
	assert_eq!(CONFIG_ROW_COUNT, 5);
}
