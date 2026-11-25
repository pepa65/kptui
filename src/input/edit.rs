use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use keepass::config::DatabaseVersion;
use zeroize::{Zeroize, Zeroizing};

use crate::app::{App, Screen, TotpCache};
use crate::db::{current_totp_code, parse_totp_edit_value, save_database_as, totp_edit_value};
use crate::input::command::preview_entry;
use crate::input::fieldedit::{EditAction, FieldEditor};

const FIELD_COUNT: usize = 6;
const NAME_INDEX: usize = 0;
const USER_INDEX: usize = 1;
const PASSWORD_INDEX: usize = 2;
const URL_INDEX: usize = 3;
const TOTP_INDEX: usize = 4;
const NOTES_INDEX: usize = 5;

fn is_reveal_key(key: &KeyEvent) -> bool {
	matches!(key.code, KeyCode::Char('r' | 'R')) && key.modifiers.contains(KeyModifiers::CONTROL)
}

pub fn handle_edit_input(app: &mut App, key: KeyEvent) {
	if app.editing_field {
		// Control-R shows or hides the password while editing it, too.
		if is_reveal_key(&key) && app.edit_state.selected() == Some(PASSWORD_INDEX) {
			app.show_password = !app.show_password;
			return;
		}
		handle_field_input(app, key);
		return;
	}

	if app.confirm_exit {
		handle_exit_confirmation(app, key.code);
		return;
	}

	match key.code {
		KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
			request_save(app);
		}
		KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) && app.tmux_copy() => copy_selected_field(app),
		_ if is_reveal_key(&key) => {
			app.status = None;
			app.show_password = !app.show_password;
		}
		_ => {
			app.status = None;
			match key.code {
				KeyCode::Esc => request_close_edit(app),
				KeyCode::Enter => start_editing_field(app),
				KeyCode::PageUp => {
					app.field_editor_scroll = app.field_editor_scroll.saturating_sub(1);
				}
				KeyCode::PageDown => {
					app.field_editor_scroll = app.field_editor_scroll.saturating_add(1);
				}
				KeyCode::Down => move_selection(app, 1),
				KeyCode::Up => move_selection(app, -1),
				_ => {}
			}
		}
	}
}

fn move_selection(app: &mut App, delta: i32) {
	let current = app.edit_state.selected().unwrap_or(0) as i32;
	let next = (current + delta).rem_euclid(FIELD_COUNT as i32) as usize;
	app.edit_state.select(Some(next));
}

fn start_editing_field(app: &mut App) {
	let selected = app.edit_state.selected().unwrap_or(0);

	let Some(entry) = app.edit_entry.as_ref() else {
		return;
	};

	match selected {
		NAME_INDEX => {
			app.field_editor = Some(FieldEditor::with_text(&entry.name));
		}

		USER_INDEX => {
			app.field_editor = Some(FieldEditor::with_text(&entry.user));
		}

		PASSWORD_INDEX => {
			app.field_editor = Some(FieldEditor::with_text(&entry.password));
		}

		URL_INDEX => {
			app.field_editor = Some(FieldEditor::with_text(&entry.url));
		}

		TOTP_INDEX => {
			app.field_editor = Some(FieldEditor::with_text(totp_edit_value(&entry.totp)));
		}

		NOTES_INDEX => {
			app.field_editor = Some(FieldEditor::with_multiline_text(&entry.notes));
		}

		_ => return,
	}

	app.editing_field = true;
	app.status = None;
}

/// Copies the selected field to a tmux paste buffer, also when the password is hidden.
fn copy_selected_field(app: &mut App) {
	let selected = app.edit_state.selected().unwrap_or(0);
	let Some(entry) = app.edit_entry.as_ref() else {
		return;
	};

	// The code on screen may have expired since the last redraw.
	if selected == TOTP_INDEX && app.totp_cache.as_ref().is_none_or(|cache| cache.expires <= Instant::now()) {
		app.totp_cache = current_totp_code(&entry.totp).map(|t| TotpCache { code: t.code, next: t.next, expires: Instant::now() + t.valid_for });
	}

	let (label, value) = match selected {
		NAME_INDEX => ("Name", entry.name.as_str()),
		USER_INDEX => ("User", entry.user.as_str()),
		PASSWORD_INDEX => ("Password", entry.password.as_str()),
		URL_INDEX => ("URL", entry.url.as_str()),
		TOTP_INDEX => ("TOTP code", app.totp_cache.as_ref().map_or("", |cache| cache.code.as_str())),
		NOTES_INDEX => ("Notes", entry.notes.as_str()),
		_ => return,
	};

	if value.is_empty() {
		app.status = Some(format!("{label} is empty, nothing copied"));
		return;
	}

	let clear_after = app.config.tmux_clear;
	app.status = Some(match app.tmux_buffer.copy(value, clear_after) {
		Ok(()) => format!("{label} copied to tmux, cleared in {}s", clear_after.as_secs()),
		Err(err) => format!("{label} {err:#}"),
	});
}

fn handle_field_input(app: &mut App, key: KeyEvent) {
	let action = match app.field_editor.as_mut() {
		Some(editor) => editor.handle_key(key),
		None => {
			app.editing_field = false;
			return;
		}
	};

	match action {
		EditAction::Continue => {}

		EditAction::Accept => commit_field(app),

		EditAction::Cancel => {
			app.editing_field = false;
			app.field_editor = None;
		}
	}
}

/// Wipes the old value, then moves the new one in without copying.
fn set_field(slot: &mut String, mut value: Zeroizing<String>) {
	slot.zeroize();
	*slot = std::mem::take(&mut *value);
}

fn commit_field(app: &mut App) {
	let selected = app.edit_state.selected().unwrap_or(0);

	let mut value = app.field_editor.take().map(|editor| editor.into_text()).unwrap_or_else(|| Zeroizing::new(String::new()));

	if selected == TOTP_INDEX {
		match parse_totp_edit_value(&value) {
			Ok(totp) => {
				if let Some(entry) = app.edit_entry.as_mut() {
					entry.totp.zeroize();
					entry.totp = totp;
				}
				app.editing_field = false;
				app.totp_cache = None;
			}
			Err(err) => {
				app.status = Some(format!("TOTP should be SECRET DIGITS PERIOD: {err}"));
				app.field_editor = Some(FieldEditor::with_text(value.as_str()));
			}
		}
		return;
	}
	if let Some(entry) = app.edit_entry.as_mut() {
		match selected {
			NAME_INDEX => {
				entry.name = if value.trim().is_empty() { String::new() } else { std::mem::take(&mut *value) };
			}
			USER_INDEX => set_field(&mut entry.user, value),
			PASSWORD_INDEX => set_field(&mut entry.password, value),
			URL_INDEX => entry.url = std::mem::take(&mut *value),
			NOTES_INDEX => set_field(&mut entry.notes, value),
			_ => {}
		}
	}
	app.editing_field = false;
}

fn request_close_edit(app: &mut App) {
	if has_unsaved_changes(app) {
		app.confirm_exit = true;
		app.status = Some("Save changes? [y] Yes  [n] No".to_string());
	} else {
		reset_edit_state(app);
	}
}

fn has_unsaved_changes(app: &App) -> bool {
	let (Some(entry), Some(original)) = (app.edit_entry.as_ref(), app.edit_original.as_ref()) else {
		return false;
	};

	entry.name != original.name
		|| entry.user != original.user
		|| entry.password != original.password
		|| entry.url != original.url
		|| entry.totp != original.totp
		|| entry.notes != original.notes
}

fn handle_exit_confirmation(app: &mut App, key: KeyCode) {
	match crate::input::normalize_shortcut(key) {
		KeyCode::Char('y') => {
			request_save(app);
		}
		KeyCode::Char('n') => {
			app.confirm_exit = false;
			reset_edit_state(app);
			app.status = Some("Changes discarded".to_string());
		}
		KeyCode::Esc => {
			app.confirm_exit = false;
			app.status = None;
		}
		_ => {}
	}
}

fn request_save(app: &mut App) {
	let version = match app.kdbx.as_ref().map(|db| &db.config.version) {
		Some(DatabaseVersion::KDB4(0)) => DatabaseVersion::KDB4(0),
		_ => DatabaseVersion::KDB4(1),
	};
	app.status = Some(format!("Saving as {version}..."));
	app.pending_save = Some(version);
}

fn reset_edit_state(app: &mut App) {
	app.edit_entry = None;
	app.edit_original = None;
	app.edit_target = None;
	app.editing_field = false;
	app.field_editor = None;
	app.field_editor_scroll = 0;
	app.confirm_exit = false;
	app.edit_state.select(None);
	app.screen = Screen::Index;
	app.totp_cache = None;
	app.show_password = false;
}

fn save_edit(app: &mut App) -> Option<usize> {
	let entry = app.edit_entry.take()?;
	let saved_idx = match app.edit_target {
		Some(idx) => {
			if let Some(slot) = app.entries.get_mut(idx) {
				*slot = entry;
			}
			Some(idx)
		}
		None => {
			if entry.name.trim().is_empty() {
				None
			} else {
				app.entries.push(entry);
				Some(app.entries.len() - 1)
			}
		}
	};
	app.refresh_filter();
	saved_idx
}

pub(crate) fn process_pending_save(app: &mut App) {
	let Some(version) = app.pending_save.take() else {
		return;
	};

	if app.screen == Screen::Settings {
		crate::input::settings::process_format_save(app, version);
		return;
	}

	let close_after_save = app.confirm_exit;
	let saved_idx = save_edit(app);
	persist_entry(app, saved_idx, version);
	reset_edit_state(app);
	if let Some(saved_idx) = saved_idx
		&& let Some(filtered_pos) = app.filtered.iter().position(|&idx| idx == saved_idx)
	{
		app.index_state.select(Some(filtered_pos));
		if !close_after_save {
			preview_entry(app);
		}
	}
}

fn persist_entry(app: &mut App, idx: Option<usize>, version: DatabaseVersion) {
	let (Some(db), Some(key), Some(path)) = (app.kdbx.as_mut(), app.db_key.as_ref(), app.config.default_database.as_ref()) else {
		app.status = Some("Not saved: database is locked".to_string());
		return;
	};

	let entries = match idx.and_then(|idx| app.entries.get_mut(idx)) {
		Some(entry) => std::slice::from_mut(entry),
		None => &mut [],
	};

	let saved_message = format!("Saved as {version}");
	match save_database_as(path, key, db, entries, version) {
		Ok(()) => app.status = Some(saved_message),
		Err(err) => app.status = Some(format!("Failed to save: {err:#}")),
	}
}

#[cfg(test)]
mod save_shortcut_tests {
	use super::*;

	#[test]
	fn confirmed_exit_writes_changes_to_disk() {
		let path = std::env::temp_dir().join(format!("kptui-confirm-save-{}.kdbx", std::process::id()));
		let password = Zeroizing::new("save-test".to_string());
		let (mut db, key, _) = crate::db::create_database(&path, 1, &password, None, crate::config::MIB).unwrap();
		db.config.version = DatabaseVersion::KDB4(0);
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		app.kdbx = Some(db);
		app.db_key = Some(key);
		app.config.default_database = Some(path.clone());

		// Cover a new entry, then updates to the existing entry in both formats.
		for minor in [0, 1] {
			app.kdbx.as_mut().unwrap().config.version = DatabaseVersion::KDB4(minor);
			app.screen = Screen::Edit;
			app.edit_target = if app.entries.is_empty() { None } else { Some(0) };
			let original = app.entries.first().cloned().unwrap_or_default();
			let mut edited = original.clone();
			edited.name = format!("Name {minor}");
			edited.user = format!("User {minor}");
			edited.password = format!("Password {minor}");
			edited.url = format!("https://example.com/{minor}");
			edited.totp = "otpauth://totp/?secret=JBSWY3DPEHPK3PXP&digits=6&period=30".to_string();
			edited.notes = format!("Notes {minor}\nSecond line");
			app.edit_original = Some(original);
			app.edit_entry = Some(edited.clone());

			handle_edit_input(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
			assert!(app.confirm_exit);
			handle_edit_input(&mut app, KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
			assert!(app.confirm_exit);
			assert_eq!(app.pending_save, Some(DatabaseVersion::KDB4(minor)));
			assert_eq!(app.status, Some(format!("Saving as KDBX4.{minor}...")));
			process_pending_save(&mut app);
			assert!(app.screen == Screen::Index);
			assert!(!app.confirm_exit);
			assert_eq!(app.status, Some(format!("Saved as KDBX4.{minor}")));

			let (reopened, _, entries) = crate::db::unlock_database(&path, &password, None).unwrap();
			assert_eq!(reopened.config.version, DatabaseVersion::KDB4(minor));
			assert_eq!(entries.len(), 1);
			let entry = &entries[0];
			assert_eq!(entry.name, edited.name);
			assert_eq!(entry.user, edited.user);
			assert_eq!(entry.password, edited.password);
			assert_eq!(entry.url, edited.url);
			assert_eq!(entry.totp, edited.totp);
			assert_eq!(entry.notes, edited.notes);
		}
		std::fs::remove_file(path).unwrap();
	}

	#[test]
	fn saves_database_when_selected_entry_is_absent() {
		let path = std::env::temp_dir().join(format!("kptui-save-without-entry-{}.kdbx", std::process::id()));
		let password = Zeroizing::new("save-test".to_string());
		let (mut db, key, _) = crate::db::create_database(&path, 1, &password, None, crate::config::MIB).unwrap();
		db.root_mut().add_entry().set_unprotected(keepass::db::fields::TITLE, "Existing entry");
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		app.kdbx = Some(db);
		app.db_key = Some(key);
		app.config.default_database = Some(path.clone());

		for minor in [0, 1] {
			if minor == 1 {
				app.edit_entry = Some(crate::db::Entry::default());
				app.edit_target = Some(100);
			}
			app.pending_save = Some(DatabaseVersion::KDB4(minor));
			process_pending_save(&mut app);
			assert_eq!(app.status, Some(format!("Saved as KDBX4.{minor}")));
			let (db, _, entries) = crate::db::unlock_database(&path, &password, None).unwrap();
			assert_eq!(db.config.version, DatabaseVersion::KDB4(minor));
			assert_eq!(entries.len(), 1);
			assert_eq!(entries[0].name, "Existing entry");
		}
		std::fs::remove_file(path).unwrap();
	}

	#[test]
	fn save_shortcut_preserves_the_current_format() {
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		for minor in [0, 1] {
			let mut db = keepass::Database::new();
			db.config.version = DatabaseVersion::KDB4(minor);
			app.kdbx = Some(db);
			handle_edit_input(&mut app, KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
			assert_eq!(app.pending_save, Some(DatabaseVersion::KDB4(minor)));
		}
	}
}
