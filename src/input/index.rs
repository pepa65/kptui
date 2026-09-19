use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::input::command::{handle_shortcut, preview_entry};

fn start_delete_confirmation(app: &mut App, index: usize) {
	if index >= app.entries.len() {
		return;
	}

	app.confirm_index_delete = Some(index);
	app.status = Some("Delete this entry? [y] Confirm  [any other key] Cancel".to_string());
}

fn handle_delete_confirmation(app: &mut App, key: KeyCode) {
	let index = app.confirm_index_delete.take();
	app.status = None;

	if let Some(index) = index
		&& let KeyCode::Char('y') = crate::input::normalize_shortcut(key)
	{
		delete_index_entry(app, index);
	}
}

fn delete_index_entry(app: &mut App, index: usize) {
	let Some(entry) = app.entries.get(index) else {
		return;
	};

	let Some(id) = entry.id else {
		return;
	};

	let (Some(db), Some(key), Some(path)) = (app.kdbx.as_mut(), app.db_key.as_ref(), app.config.default_database.as_ref()) else {
		app.status = Some("Not deleted: database is locked".to_string());
		return;
	};

	if let Err(err) = crate::db::delete_entry(path, key, db, id) {
		app.status = Some(format!("Failed to delete: {err:#}"));
		return;
	}

	app.entries.remove(index);
	app.refresh_filter();
	app.status = Some("Entry deleted".to_string());
}

fn move_selection_to(app: &mut App, selected: usize) {
	if app.filtered.is_empty() {
		return;
	}

	app.status = None;
	app.index_state.select(Some(selected.min(app.filtered.len() - 1)));
}

pub fn handle_index_input(app: &mut App, key: KeyEvent) {
	if app.confirm_index_delete.is_some() {
		handle_delete_confirmation(app, key.code);
		return;
	}

	if key.modifiers.contains(KeyModifiers::CONTROL) {
		if let KeyCode::Char(c) = key.code {
			app.status = None;
			if c.eq_ignore_ascii_case(&'d') {
				if let Some(selected) = app.index_state.selected()
					&& let Some(&index) = app.filtered.get(selected)
				{
					start_delete_confirmation(app, index);
				}
			} else {
				handle_shortcut(app, c.to_ascii_lowercase());
			}
		}
		return;
	}

	match key.code {
		KeyCode::Char(c) => {
			app.status = None;

			if app.query.len() < app.max_len {
				app.query.push(c);
				app.refresh_filter();
			}
		}

		KeyCode::Backspace => {
			app.status = None;
			app.query.pop();
			app.refresh_filter();
		}

		KeyCode::Enter => {
			app.status = None;
			preview_entry(app);
		}

		KeyCode::Esc => {
			app.should_quit = true;
		}

		KeyCode::Down => {
			let row_count = app.filtered.len();

			if row_count == 0 {
				return;
			}

			app.status = None;

			let selected = app.index_state.selected().unwrap_or(0);

			let next = if selected >= row_count - 1 { 0 } else { selected + 1 };

			app.index_state.select(Some(next));
		}

		KeyCode::Up => {
			let row_count = app.filtered.len();

			if row_count == 0 {
				return;
			}

			app.status = None;

			let selected = app.index_state.selected().unwrap_or(0);

			let prev = if selected == 0 { row_count - 1 } else { selected - 1 };

			app.index_state.select(Some(prev));
		}

		KeyCode::PageDown => {
			let row_count = app.filtered.len();
			if row_count == 0 {
				return;
			}

			app.status = None;
			let selected = app.index_state.selected().unwrap_or(0);
			let next = selected.saturating_add(app.index_page_size).min(row_count - 1);
			app.index_state.select(Some(next));
		}

		KeyCode::PageUp => {
			let row_count = app.filtered.len();
			if row_count == 0 {
				return;
			}

			app.status = None;
			let selected = app.index_state.selected().unwrap_or(0);
			let prev = selected.saturating_sub(app.index_page_size);
			app.index_state.select(Some(prev));
		}

		KeyCode::Home => {
			move_selection_to(app, 0);
		}

		KeyCode::End if !app.filtered.is_empty() => {
			move_selection_to(app, app.filtered.len() - 1);
		}

		_ => {}
	}
}
