use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::input::command::{handle_shortcut, preview_entry};
use crate::input::{secret_pop, secret_push};
use zeroize::Zeroize;

fn start_delete_confirmation(app: &mut App, index: usize) {
	if index >= app.entries.len() {
		return;
	}

	app.confirm_index_delete = Some(index);
	app.status = Some("Delete this entry? [y] Confirm and save  [Esc] Cancel".to_string());
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

	let id = entry.id;

	let (Some(db), Some(key), Some(path)) = (app.kdbx.as_mut(), app.db_key.as_ref(), app.config.default_database.as_ref()) else {
		app.status = Some("Not deleted: database is locked".to_string());
		return;
	};

	let result = match id {
		Some(id) => crate::db::delete_entry(path, key, db, id),
		None => crate::db::save_database(path, key, db, &mut []),
	};
	if let Err(err) = result {
		app.status = Some(format!("Failed to delete: {err:#}"));
		return;
	}

	app.entries.remove(index);
	app.refresh_filter();
	app.status = Some("Entry deleted".to_string());
}

fn move_selection_to(app: &mut App, selected: usize) {
	let row_count = app.row_count();
	if row_count == 0 {
		return;
	}

	app.status = None;
	app.index_state.select(Some(selected.min(row_count - 1)));
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
				if let Some(index) = app.selected_entry() {
					start_delete_confirmation(app, index);
				} else if app.selected_group().is_some() {
					app.status = Some("Groups can't be deleted in kptui".to_string());
				}
			} else if c.eq_ignore_ascii_case(&'f') {
				app.search_notes = !app.search_notes;
				app.refresh_filter();
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
				secret_push(&mut app.query, c);
				app.refresh_filter();
			}
		}

		KeyCode::Backspace => {
			app.status = None;
			secret_pop(&mut app.query);
			app.refresh_filter();
		}

		KeyCode::Enter => {
			app.status = None;
			match app.selected_group() {
				Some(group) => app.enter_group(group),
				None => preview_entry(app),
			}
		}

		KeyCode::Right => {
			if let Some(group) = app.selected_group() {
				app.status = None;
				app.enter_group(group);
			}
		}

		// With groups, Esc first clears the search, then goes up a level, then quits.
		KeyCode::Esc if !app.groups.is_empty() && !app.query.is_empty() => {
			app.status = None;
			app.query.zeroize();
			app.refresh_filter();
		}

		KeyCode::Esc if app.group.is_some() => {
			app.status = None;
			app.leave_group();
		}

		KeyCode::Left if app.query.is_empty() => {
			app.status = None;
			app.leave_group();
		}

		KeyCode::Esc => {
			app.should_quit = true;
		}

		KeyCode::Down => {
			let row_count = app.row_count();

			if row_count == 0 {
				return;
			}

			app.status = None;

			let selected = app.index_state.selected().unwrap_or(0);

			let next = if selected >= row_count - 1 { 0 } else { selected + 1 };

			app.index_state.select(Some(next));
		}

		KeyCode::Up => {
			let row_count = app.row_count();

			if row_count == 0 {
				return;
			}

			app.status = None;

			let selected = app.index_state.selected().unwrap_or(0);

			let prev = if selected == 0 { row_count - 1 } else { selected - 1 };

			app.index_state.select(Some(prev));
		}

		KeyCode::PageDown => {
			let row_count = app.row_count();
			if row_count == 0 {
				return;
			}

			app.status = None;
			let selected = app.index_state.selected().unwrap_or(0);
			let next = selected.saturating_add(app.index_page_size).min(row_count - 1);
			app.index_state.select(Some(next));
		}

		KeyCode::PageUp => {
			let row_count = app.row_count();
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

		KeyCode::End if app.row_count() > 0 => {
			move_selection_to(app, app.row_count() - 1);
		}

		_ => {}
	}
}
