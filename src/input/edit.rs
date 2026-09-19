use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, Screen};
use crate::db::save_database;
use crate::input::command::preview_entry;
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

	match key.code {
		KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
			save_and_reopen_edit(app);
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

fn save_and_reopen_edit(app: &mut App) {
	let Some(saved_idx) = save_edit(app) else {
		reset_edit_state(app);
		return;
	};

	reset_edit_state(app);
	let Some(filtered_pos) = app.filtered.iter().position(|&idx| idx == saved_idx) else {
		return;
	};

	app.index_state.select(Some(filtered_pos));
	preview_entry(app);
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
	app.field_editor_scroll = 0;
	app.confirm_exit = false;
	app.edit_state.select(None);
	app.screen = Screen::Index;
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
	if let Some(idx) = saved_idx {
		persist_entry(app, idx);
	}
	saved_idx
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
