use crate::app::App;
use crate::app::Screen::{Edit, Settings};
use crate::db::Entry;
use crate::input::fieldedit::FieldEditor;

pub fn handle_shortcut(app: &mut App, c: char) {
	match c {
		'a' => add_entry(app),
		'c' => open_settings(app),
		_ => {}
	}
}

fn add_entry(app: &mut App) {
	app.edit_entry = Some(Entry::default());
	app.edit_original = Some(Entry::default());
	app.edit_target = None;
	app.edit_state.select(Some(0));
	app.field_editor = Some(FieldEditor::with_text(""));
	app.editing_field = true;
	app.confirm_delete = false;
	app.confirm_exit = false;
	app.screen = Edit;
}

pub fn preview_entry(app: &mut App) {
	let Some(selected) = app.index_state.selected() else {
		return;
	};
	let Some(&entry_idx) = app.filtered.get(selected) else {
		return;
	};
	let Some(entry) = app.entries.get(entry_idx) else {
		return;
	};

	app.edit_entry = Some(entry.clone());
	app.edit_original = Some(entry.clone());
	app.edit_target = Some(entry_idx);
	app.edit_state.select(Some(0));
	app.confirm_delete = false;
	app.confirm_exit = false;
	app.screen = Edit;
}

fn open_settings(app: &mut App) {
	app.status = None;
	app.available_themes = crate::theme::list_theme_names().unwrap_or_default();

	let current_idx = app.config.theme.as_deref().and_then(|current| {
		let current = crate::theme::slugify(current);
		app.available_themes.iter().position(|name| crate::theme::slugify(name) == current)
	});

	let selected = current_idx.or(if app.available_themes.is_empty() { None } else { Some(0) });

	app.settings_state.select(selected);
	app.screen = Settings;
}
