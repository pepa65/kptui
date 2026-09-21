use std::time::Instant;

use crossterm::event::KeyCode;
use zeroize::Zeroize;

use crate::app::{App, Screen};
use crate::config::save_config;
use crate::db::{create_database, unlock_database};
use crate::util::{default_new_database_path, secret_pop};

pub fn handle_login_input(app: &mut App, key: KeyCode) {
	if app.creating_database {
		handle_create_database_input(app, key);
		return;
	}

	if database_missing(app) {
		handle_missing_database_input(app, key);
		return;
	}

	match key {
		KeyCode::Char(c) => {
			app.login_error = None;

			if app.password.len() < app.max_len {
				app.password.push(c);
			}
		}

		KeyCode::Backspace => {
			app.login_error = None;
			secret_pop(&mut app.password);
		}

		KeyCode::Enter => {
			attempt_unlock(app);
		}

		KeyCode::Esc => {
			app.should_quit = true;
		}

		_ => {}
	}
}

pub fn database_missing(app: &App) -> bool {
	match &app.config.default_database {
		None => true,
		Some(path) => !path.is_file(),
	}
}

fn handle_missing_database_input(app: &mut App, key: KeyCode) {
	match crate::input::normalize_shortcut(key) {
		KeyCode::Char('c') => {
			if app.password.is_empty() {
				start_create_database(app);
			} else {
				create_database_with_password(app);
			}
		}
		KeyCode::Esc => {
			app.should_quit = true;
		}
		_ => {}
	}
}

fn start_create_database(app: &mut App) {
	app.creating_database = true;
	app.confirming_new_db_password = false;
	app.password.zeroize();
	app.new_db_confirm.zeroize();
	app.login_error = None;
}

fn cancel_create_database(app: &mut App) {
	app.creating_database = false;
	app.confirming_new_db_password = false;
	app.password.zeroize();
	app.new_db_confirm.zeroize();
	app.login_error = None;
}

fn handle_create_database_input(app: &mut App, key: KeyCode) {
	match key {
		KeyCode::Char(c) => {
			app.login_error = None;
			let limit = app.max_len;

			if app.confirming_new_db_password {
				if app.new_db_confirm.len() < limit {
					app.new_db_confirm.push(c);
				}
			} else if app.password.len() < limit {
				app.password.push(c);
			}
		}

		KeyCode::Backspace => {
			app.login_error = None;

			if app.confirming_new_db_password {
				secret_pop(&mut app.new_db_confirm);
			} else {
				secret_pop(&mut app.password);
			}
		}

		KeyCode::Enter => {
			if app.confirming_new_db_password {
				finish_create_database(app);
			} else if app.password.is_empty() {
				app.login_error = Some("Master password can't be empty".to_string());
			} else {
				app.confirming_new_db_password = true;
			}
		}

		KeyCode::Esc => cancel_create_database(app),

		_ => {}
	}
}

fn finish_create_database(app: &mut App) {
	if app.new_db_confirm != app.password {
		app.login_error = Some("Passwords don't match".to_string());
		app.new_db_confirm.zeroize();
		app.confirming_new_db_password = false;
		return;
	}

	create_database_with_password(app);
}

pub fn create_database_with_password(app: &mut App) {
	let path = app.config.default_database.clone().unwrap_or_else(default_new_database_path);

	match create_database(&path, &app.password, app.config.keyfile.as_deref()) {
		Ok((db, key, entries)) => {
			app.config.default_database = Some(path.clone());
			let save_result = save_config(&app.config);

			app.kdbx = Some(db);
			app.db_key = Some(key);
			app.entries = entries;
			app.password.zeroize();
			app.new_db_confirm.zeroize();
			app.creating_database = false;
			app.confirming_new_db_password = false;
			app.login_error = None;
			app.last_activity = Instant::now();
			app.refresh_filter();
			app.screen = Screen::Index;

			app.status = Some(match save_result {
				Ok(()) => format!("Created new database at {}", path.display()),
				Err(err) => format!("Database created, but couldn't save config: {err}"),
			});
		}

		Err(err) => {
			app.login_error = Some(format!("Couldn't create database: {err:#}"));
			app.password.zeroize();
			app.new_db_confirm.zeroize();
			app.confirming_new_db_password = false;
		}
	}
}

pub fn attempt_unlock(app: &mut App) {
	let Some(path) = app.config.default_database.clone() else {
		app.login_error = Some("No default_database set in configfile".to_string());
		return;
	};

	match unlock_database(&path, &app.password, app.config.keyfile.as_deref()) {
		Ok((db, key, entries)) => {
			app.entries = entries;
			app.kdbx = Some(db);
			app.db_key = Some(key);
			app.password.zeroize();
			app.login_error = None;
			app.last_activity = Instant::now();
			app.refresh_filter();
			app.screen = Screen::Index;
		}
		Err(err) => {
			app.password.zeroize();
			app.login_error = Some(format!("{err}"));
		}
	}
}
