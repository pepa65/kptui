//! The additional attributes of the open entry: custom fields with a name and
//! a value, which can be protected, like in KeePassXC's Advanced section.
//!
//! Changes go into the open entry, and get saved with it (Control-S).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use zeroize::Zeroize;

use crate::app::{App, AttributeEdit};
use crate::db::{Attribute, is_standard_field, sort_attributes};
use crate::input::fieldedit::{EditAction, FieldEditor};

/// Shows the attributes of the open entry.
pub fn open(app: &mut App) {
	let count = app.edit_entry.as_ref().map_or(0, |entry| entry.attributes.len());
	app.attributes_open = true;
	app.attribute_state.select((count > 0).then_some(0));
	app.attribute_scroll = 0;
	app.show_protected = false;
	app.status = None;
	stop_editing(app);
}

/// Goes back to the entry; also used when the entry gets closed.
pub fn close(app: &mut App) {
	app.attributes_open = false;
	app.show_protected = false;
	stop_editing(app);
}

/// Stops editing a name or value. The field editor is only reset when it was
/// the panel's, as closing the panel happens when an entry gets opened, too.
fn stop_editing(app: &mut App) {
	if app.attribute_edit.take().is_some() {
		app.editing_field = false;
		app.field_editor = None;
	}
	app.confirm_attribute_delete = false;
}

fn attributes(app: &App) -> &[Attribute] {
	app.edit_entry.as_ref().map_or(&[], |entry| entry.attributes.as_slice())
}

fn selected(app: &App) -> Option<usize> {
	app.attribute_state.selected().filter(|&index| index < attributes(app).len())
}

pub fn handle_attributes_input(app: &mut App, key: KeyEvent) {
	if app.attribute_edit.is_some() {
		handle_editor_input(app, key);
		return;
	}

	if app.confirm_attribute_delete {
		app.confirm_attribute_delete = false;
		app.status = None;
		if let KeyCode::Char('y') = crate::input::normalize_shortcut(key.code) {
			delete_selected(app);
		}
		return;
	}

	app.status = None;
	if key.modifiers.contains(KeyModifiers::CONTROL) {
		if let KeyCode::Char(c) = key.code {
			match c.to_ascii_lowercase() {
				'a' => start_editing(app, AttributeEdit::Name(None)),
				'n' => {
					if let Some(index) = selected(app) {
						start_editing(app, AttributeEdit::Name(Some(index)));
					}
				}
				'p' => toggle_protection(app),
				'r' => app.show_protected = !app.show_protected,
				'd' => {
					if let Some(index) = selected(app) {
						app.confirm_attribute_delete = true;
						app.status = Some(format!("Delete \"{}\"? [y] Confirm  [any other key] Cancel", attributes(app)[index].name));
					}
				}
				's' => crate::input::edit::request_save(app),
				_ => {}
			}
		}
		return;
	}

	let count = attributes(app).len();
	match key.code {
		KeyCode::Esc => close(app),
		KeyCode::Enter => {
			if let Some(index) = selected(app) {
				start_editing(app, AttributeEdit::Value(index));
			}
		}
		KeyCode::Down | KeyCode::Up | KeyCode::Home | KeyCode::End if count > 0 => {
			let current = selected(app).unwrap_or(0);
			let next = match key.code {
				KeyCode::Down => (current + 1) % count,
				KeyCode::Up => (current + count - 1) % count,
				KeyCode::Home => 0,
				_ => count - 1,
			};
			app.attribute_state.select(Some(next));
			app.attribute_scroll = 0;
		}
		KeyCode::PageDown => app.attribute_scroll = app.attribute_scroll.saturating_add(1),
		KeyCode::PageUp => app.attribute_scroll = app.attribute_scroll.saturating_sub(1),
		_ => {}
	}
}

fn start_editing(app: &mut App, edit: AttributeEdit) {
	let editor = match edit {
		AttributeEdit::Name(None) => FieldEditor::default(),
		AttributeEdit::Name(Some(index)) => FieldEditor::with_text(attributes(app)[index].name.as_str()),
		AttributeEdit::Value(index) => FieldEditor::with_multiline_text(attributes(app)[index].value.as_str()),
	};
	app.field_editor = Some(editor);
	app.editing_field = true;
	app.attribute_edit = Some(edit);
	app.attribute_scroll = 0;
}

fn handle_editor_input(app: &mut App, key: KeyEvent) {
	// Control-R shows or hides a protected value while editing it, too.
	if matches!(key.code, KeyCode::Char('r' | 'R')) && key.modifiers.contains(KeyModifiers::CONTROL) {
		if matches!(app.attribute_edit, Some(AttributeEdit::Value(_))) {
			app.show_protected = !app.show_protected;
		}
		return;
	}

	let Some(editor) = app.field_editor.as_mut() else {
		stop_editing(app);
		return;
	};
	match editor.handle_key(key) {
		EditAction::Continue => app.status = None,
		EditAction::Cancel => {
			app.status = None;
			stop_editing(app);
		}
		EditAction::Accept => commit(app),
	}
}

fn commit(app: &mut App) {
	let Some(edit) = app.attribute_edit else {
		stop_editing(app);
		return;
	};
	// A name that isn't valid stays in the editor as typed, to fix it.
	if let AttributeEdit::Name(target) = edit
		&& let Err(err) = check_name(app.field_editor.as_ref().map_or("", FieldEditor::text), attributes(app), target)
	{
		app.status = Some(err);
		return;
	}
	let Some(editor) = app.field_editor.take() else {
		stop_editing(app);
		return;
	};
	let mut text = editor.into_text();

	match edit {
		AttributeEdit::Value(index) => {
			if let Some(attribute) = app.edit_entry.as_mut().and_then(|entry| entry.attributes.get_mut(index)) {
				// Wipe the old value, then move the new one in without copying.
				attribute.value.zeroize();
				attribute.value = std::mem::take(&mut *text);
			}
			app.status = None;
			stop_editing(app);
		}
		AttributeEdit::Name(target) => match check_name(&text, attributes(app), target) {
			Ok(name) => {
				let Some(entry) = app.edit_entry.as_mut() else {
					stop_editing(app);
					return;
				};
				match target {
					Some(index) => entry.attributes[index].name = name.clone(),
					None => entry.attributes.push(Attribute { name: name.clone(), value: String::new(), protected: false }),
				}
				sort_attributes(&mut entry.attributes);
				let index = entry.attributes.iter().position(|attribute| attribute.name == name);
				app.attribute_state.select(index);
				app.status = None;
				stop_editing(app);
				// A new attribute goes on to its value right away.
				if let (None, Some(index)) = (target, index) {
					start_editing(app, AttributeEdit::Value(index));
				}
			}
			// Checked above already.
			Err(err) => {
				app.status = Some(err);
				stop_editing(app);
			}
		},
	}
}

/// Checks a new name for the attribute at `target` (`None`: a new one).
fn check_name(name: &str, attributes: &[Attribute], target: Option<usize>) -> Result<String, String> {
	let name = name.trim();
	if name.is_empty() {
		return Err("The name can't be empty".to_string());
	}
	if name.chars().any(char::is_control) {
		return Err("The name can't contain control characters".to_string());
	}
	if is_standard_field(name) {
		return Err(format!("\"{name}\" is a standard field"));
	}
	if attributes.iter().enumerate().any(|(index, attribute)| Some(index) != target && attribute.name == name) {
		return Err(format!("There already is an attribute \"{name}\""));
	}
	Ok(name.to_string())
}

fn toggle_protection(app: &mut App) {
	let Some(index) = selected(app) else {
		return;
	};
	let Some(attribute) = app.edit_entry.as_mut().and_then(|entry| entry.attributes.get_mut(index)) else {
		return;
	};
	attribute.protected = !attribute.protected;
	app.status = Some(format!("\"{}\" is {}protected now", attribute.name, if attribute.protected { "" } else { "not " }));
}

fn delete_selected(app: &mut App) {
	let Some(index) = selected(app) else {
		return;
	};
	let Some(entry) = app.edit_entry.as_mut() else {
		return;
	};
	// Dropping the attribute wipes it.
	entry.attributes.remove(index);
	let count = entry.attributes.len();
	app.attribute_state.select(if count == 0 { None } else { Some(index.min(count - 1)) });
	app.attribute_scroll = 0;
}

#[cfg(test)]
mod tests {
	use super::check_name;
	use crate::db::Attribute;

	#[test]
	fn names_must_be_new_and_not_standard_fields() {
		let attribute = |name: &str| Attribute { name: name.to_string(), value: String::new(), protected: false };
		let attributes = vec![attribute("PIN"), attribute("Recovery codes")];

		assert_eq!(check_name("  Account ID ", &attributes, None), Ok("Account ID".to_string()));
		assert_eq!(check_name("PIN", &attributes, Some(0)), Ok("PIN".to_string()), "keeping its own name");
		assert_eq!(check_name("pin", &attributes, None), Ok("pin".to_string()), "names are case-sensitive, like in KeePass");
		assert!(check_name("PIN", &attributes, None).is_err());
		assert!(check_name("PIN", &attributes, Some(1)).is_err());
		assert!(check_name("   ", &attributes, None).is_err());
		assert!(check_name("a\tb", &attributes, None).is_err());
		for standard in ["Title", "UserName", "Password", "URL", "Notes", "otp"] {
			assert!(check_name(standard, &attributes, None).is_err(), "{standard}");
		}
	}
}
