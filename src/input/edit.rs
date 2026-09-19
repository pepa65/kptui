use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, Screen};
use crate::db::{calculate_warnings, delete_entry, save_database};
use crate::input::fieldedit::{EditAction, FieldEditor};

const FIELD_COUNT: usize = 7;
const NAME_INDEX: usize = 0;
const USER_INDEX: usize = 1;
const PASSWORD_INDEX: usize = 2;
const URL_INDEX: usize = 3;
const TOTP_INDEX: usize = 4;
const LAST_MODIFIED_INDEX: usize = 5;
const NOTES_INDEX: usize = 6;

pub fn handle_edit_input(app: &mut App, key: KeyEvent) {
	if app.editing_field {
		handle_field_input(app, key);
		return;
	}

	if app.confirm_exit {
		handle_exit_confirmation(app, key.code);
		return;
	}

	if app.confirm_delete {
		handle_delete_confirmation(app, key.code);
		return;
	}

	match crate::input::normalize_shortcut(key.code) {
		KeyCode::Esc => request_close_edit(app),

		KeyCode::Enter => start_editing_field(app),

		KeyCode::Char('v') => {
			app.reveal_password = !app.reveal_password;
		}

		KeyCode::Char('d') => start_delete_confirmation(app),

		KeyCode::Down => move_selection(app, 1),
		KeyCode::Up => move_selection(app, -1),

		_ => {}
	}
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

fn move_selection(app: &mut App, delta: i32) {
	let current = app.edit_state.selected().unwrap_or(0) as i32;
	let next = (current + delta).rem_euclid(FIELD_COUNT as i32) as usize;
	app.edit_state.select(Some(next));
}

fn start_editing_field(app: &mut App) {
	let selected = app.edit_state.selected().unwrap_or(0);

	if selected == LAST_MODIFIED_INDEX {
		app.status = Some("Last modified is set automatically".to_string());
		return;
	}

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
			app.reveal_password = true;
		}

		URL_INDEX => {
			app.field_editor = Some(FieldEditor::with_text(&entry.url));
		}

		TOTP_INDEX => {
			app.field_editor = Some(FieldEditor::with_text(&entry.totp));
		}

		NOTES_INDEX => {
			app.field_editor = Some(FieldEditor::with_multiline_text(&entry.notes));
		}

		_ => return,
	}

	app.editing_field = true;
	app.status = None;
}

fn commit_field(app: &mut App) {
	let selected = app.edit_state.selected().unwrap_or(0);

	let value = app.field_editor.take().map(|editor| editor.into_text()).unwrap_or_default();

	if let Some(entry) = app.edit_entry.as_mut() {
		match selected {
			NAME_INDEX => entry.name = value,
			USER_INDEX => entry.user = value,
			PASSWORD_INDEX => entry.password = value,
			URL_INDEX => entry.url = value,
			TOTP_INDEX => entry.totp = value,
			NOTES_INDEX => entry.notes = value,
			LAST_MODIFIED_INDEX => {}
			_ => {}
		}
	}

	app.editing_field = false;
}

fn request_close_edit(app: &mut App) {
	if has_unsaved_changes(app) {
		app.confirm_exit = true;
		app.status = Some("Save changes? [y] yes  [n] no".to_string());
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
			app.confirm_exit = false;
			close_edit(app);
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

fn close_edit(app: &mut App) {
	save_edit(app);
	reset_edit_state(app);
}

fn reset_edit_state(app: &mut App) {
	app.edit_entry = None;
	app.edit_original = None;
	app.edit_target = None;
	app.editing_field = false;
	app.field_editor = None;
	app.confirm_delete = false;
	app.confirm_exit = false;
	app.reveal_password = false;
	app.edit_state.select(None);
	app.screen = Screen::Index;
}

fn save_edit(app: &mut App) {
	let Some(entry) = app.edit_entry.take() else {
		return;
	};

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

	calculate_warnings(&mut app.entries);
	app.refresh_filter();

	if let Some(idx) = saved_idx {
		persist_entry(app, idx);
	}
}

fn persist_entry(app: &mut App, idx: usize) {
	let (Some(db), Some(key), Some(path)) = (app.kdbx.as_mut(), app.db_key.as_ref(), app.config.default_database.as_ref()) else {
		app.status = Some("Not saved: database is locked".to_string());
		return;
	};

	let Some(target) = app.entries.get_mut(idx) else {
		return;
	};

	match save_database(path, key, db, std::slice::from_mut(target)) {
		Ok(()) => app.status = Some("Saved".to_string()),
		Err(err) => app.status = Some(format!("Failed to save: {err:#}")),
	}
}

fn start_delete_confirmation(app: &mut App) {
	if app.edit_entry.is_none() {
		return;
	}

	app.confirm_delete = true;
	app.status = Some("Delete this entry? [y] confirm  [any other key] cancel".to_string());
}

fn handle_delete_confirmation(app: &mut App, key: KeyCode) {
	app.confirm_delete = false;
	app.status = None;

	if let KeyCode::Char('y') = crate::input::normalize_shortcut(key) {
		delete_current_entry(app);
	}
}

fn delete_current_entry(app: &mut App) {
	let Some(entry) = app.edit_entry.take() else {
		reset_edit_state(app);
		return;
	};

	if let Some(id) = entry.id {
		let (Some(db), Some(key), Some(path)) = (app.kdbx.as_mut(), app.db_key.as_ref(), app.config.default_database.as_ref()) else {
			app.status = Some("Not deleted: database is locked".to_string());
			app.edit_entry = Some(entry);
			return;
		};

		if let Err(err) = delete_entry(path, key, db, id) {
			app.status = Some(format!("Failed to delete: {err:#}"));
			app.edit_entry = Some(entry);
			return;
		}
	}

	if let Some(idx) = app.edit_target
		&& idx < app.entries.len()
	{
		app.entries.remove(idx);
	}

	calculate_warnings(&mut app.entries);
	app.refresh_filter();

	reset_edit_state(app);
	app.status = Some("Entry deleted".to_string());
}
