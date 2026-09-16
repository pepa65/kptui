use ratatui::{
	Frame,
	layout::{Constraint, Direction, Layout, Margin, Rect},
	style::Style,
	text::{Line, Span},
	widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Widget},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::App;
use crate::db::current_totp_code;
use crate::util::wrap_help_items;

const NAME_INDEX: usize = 0;
const USER_INDEX: usize = 1;
const PASSWORD_INDEX: usize = 2;
const URL_INDEX: usize = 3;
const TOTP_INDEX: usize = 4;
const LAST_MODIFIED_INDEX: usize = 5;
const NOTES_INDEX: usize = 6;

fn nav_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode {
		&["[↑↓]", "[Enter]", "[v]", "[d]", "[Esc]"]
	} else {
		&["[↑↓] navigate", "[Enter] edit field", "[v] toggle visibility", "[d] delete entry", "[Esc] close"]
	}
}

fn field_help_items(slim_mode: bool) -> &'static [&'static str] {
	if slim_mode { &["[Enter]", "[Esc]"] } else { &["[^n] newline", "[Enter] save field", "[Esc] cancel"] }
}

fn wrap_notes(notes: &str, width: usize) -> Vec<String> {
	let width = width.max(1);
	let mut wrapped = Vec::new();

	for line in notes.split('\n') {
		let line = line.strip_suffix('\r').unwrap_or(line);
		let mut current = String::new();

		for word in line.split_whitespace() {
			let separator_width = usize::from(!current.is_empty());

			if UnicodeWidthStr::width(current.as_str()) + separator_width + UnicodeWidthStr::width(word) <= width {
				if !current.is_empty() {
					current.push(' ');
				}
				current.push_str(word);
				continue;
			}

			if !current.is_empty() {
				wrapped.push(std::mem::take(&mut current));
			}

			for character in word.chars() {
				let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
				if !current.is_empty() && UnicodeWidthStr::width(current.as_str()) + character_width > width {
					wrapped.push(std::mem::take(&mut current));
				}
				current.push(character);
			}
		}

		if !current.is_empty() {
			wrapped.push(current);
		} else if line.trim().is_empty() {
			wrapped.push(String::new());
		}
	}

	wrapped
}

pub fn draw_edit(frame: &mut Frame, app: &mut App) {
	let full_area = frame.area();

	let Some(entry) = app.edit_entry.as_ref() else {
		return;
	};

	let name = entry.name.clone();
	let user = entry.user.clone();
	let url = entry.url.clone();
	let totp_raw = entry.totp.clone();
	let notes = entry.notes.clone();
	let last_modified = entry.date_last_modify.clone();
	let duplicate_user_count = entry.duplicate_user_count;
	let password_reuse_count = entry.password_reuse_count;
	let password_is_set = !entry.password.is_empty();

	let password_text = if !password_is_set {
		String::new()
	} else if app.reveal_password {
		entry.password.clone()
	} else {
		"•".repeat(entry.password.chars().count().max(8))
	};

	let is_new_entry = app.edit_target.is_none();
	let editing_field = app.editing_field;
	let selected = app.edit_state.selected().unwrap_or(0);

	let theme = &app.theme;
	let label_style = Style::new().fg(theme.header).bold();
	let normal = Style::new().fg(theme.text);
	let warning = Style::new().fg(theme.warning);
	let placeholder = Style::new().fg(theme.text).bold();
	let editing_style = Style::new().fg(theme.selection_fg).bg(theme.selection_bg);
	let accent_style = Style::new().fg(theme.selection_fg).bg(theme.accent);
	let border_style = Style::new().fg(theme.border);

	let help_items = if editing_field { field_help_items(app.slim_mode) } else { nav_help_items(app.slim_mode) };

	let help_width = full_area.width.saturating_sub(2);
	let help_lines = wrap_help_items(help_items, help_width);
	let help_height = help_lines.len() as u16;

	let vertical = Layout::default()
		.direction(Direction::Vertical)
		.constraints([Constraint::Fill(1), Constraint::Length(help_height)])
		.split(full_area);

	// The Notes area is outside the List. Calculate its geometry first,
	// because both normal-mode wrapping and textarea rendering must use
	// exactly the same width.
	let list_area = vertical[0];

	let inner = list_area.inner(Margin { horizontal: 1, vertical: 1 });

	// Six compact fields come before Notes:
	//   0 Name
	//   1 User
	//   2 Password
	//   3 URL
	//   4 TOTP
	//   5 Last modified
	// Each occupies exactly one terminal row.
	let notes_label_y = inner.y + 6;

	let notes_label_area = Rect { x: inner.x + 1, y: notes_label_y, width: inner.width.saturating_sub(1), height: 1 };

	let notes_area = Rect {
		x: inner.x + 1,
		y: notes_label_y + 1,
		width: inner.width.saturating_sub(1),
		height: inner.height.saturating_sub(7).max(1),
	};

	// Normal-mode wrapping uses exactly the same width as the editor.
	let notes_width = notes_area.width.saturating_sub(1) as usize;

	let notes_lines: Vec<Line<'static>> = if notes.is_empty() {
		vec![Line::from(Span::styled("(empty)", placeholder))]
	} else {
		wrap_notes(&notes, notes_width).into_iter().map(|line| Line::from(Span::styled(line, normal))).collect()
	};

	// Compact single-line field.
	// The actual editor is rendered as a TextArea overlay below,
	// so the List only renders the normal field contents.
	let compact_field = |field_index: usize, label: &'static str, value: String, extra: Vec<Span<'static>>| -> ListItem<'static> {
		let label_style = if selected == field_index { editing_style } else { label_style };

		let mut line = vec![Span::styled(label, label_style), Span::styled(": ", normal)];

		if value.is_empty() && extra.is_empty() {
			line.push(Span::styled("(empty)", placeholder));
		} else {
			line.push(Span::styled(value, normal));
			line.extend(extra);
		}

		ListItem::new(Line::from(line))
	};

	let user_extra = if duplicate_user_count > 1 { vec![Span::styled(format!(" [{duplicate_user_count}]"), warning)] } else { vec![] };

	let mut password_extra = if password_reuse_count > 1 { vec![Span::styled(format!(" [{password_reuse_count}]"), warning)] } else { vec![] };

	if password_is_set && app.reveal_password {
		password_extra.push(Span::styled("  [visible]", warning));
	}

	// Viewing:  TOTP: 123456 (12s)  otpauth://...
	// Editing:  TOTP: otpauth://...
	let totp_extra = if editing_field && selected == TOTP_INDEX {
		vec![]
	} else {
		match current_totp_code(&totp_raw) {
			Some(totp) => {
				let mut extra = vec![Span::styled(totp.code, normal.bold()), Span::styled(format!(" ({}s)", totp.valid_for.as_secs() + 1), warning)];

				if !totp_raw.is_empty() {
					extra.push(Span::styled(format!("  {totp_raw}"), normal));
				}

				extra
			}

			None if totp_raw.trim().is_empty() => {
				vec![]
			}

			None => {
				vec![Span::styled("couldn't generate a code", warning), Span::styled(format!("  {totp_raw}"), normal)]
			}
		}
	};

	let totp_value = String::new();

	let items = vec![
		compact_field(NAME_INDEX, "Name", name, vec![]),
		compact_field(USER_INDEX, "User", user, user_extra),
		compact_field(PASSWORD_INDEX, "Password", password_text, password_extra),
		compact_field(URL_INDEX, "URL", url, vec![]),
		compact_field(TOTP_INDEX, "TOTP", totp_value, totp_extra),
		compact_field(LAST_MODIFIED_INDEX, "Last modified", last_modified, vec![]),
	];

	let title = if is_new_entry { " New Entry " } else { " Entry " };

	let list = List::new(items).block(Block::default().borders(Borders::ALL).padding(Padding::horizontal(1)).border_style(border_style).title(title));

	// Notes is logical field 6 but is not part of the List.
	// Give the List a local copy of the state so selecting Notes does
	// not attempt to select a nonexistent seventh List item.
	let mut list_state = app.edit_state;
	if selected == NOTES_INDEX {
		list_state.select(None);
	} else {
		list_state.select(Some(selected));
	}
	frame.render_stateful_widget(list, list_area, &mut list_state);
	// Overlay the single-line textarea when editing Name/User/Password/URL/TOTP.
	if editing_field && selected <= TOTP_INDEX {
		if let Some(textarea) = app.field_textarea.as_mut() {
			textarea.set_cursor_style(accent_style);

			let inner = vertical[0].inner(Margin { horizontal: 1, vertical: 1 });

			let label = match selected {
				NAME_INDEX => "Name: ",
				USER_INDEX => "User: ",
				PASSWORD_INDEX => "Password: ",
				URL_INDEX => "URL: ",
				TOTP_INDEX => "TOTP: ",
				_ => "",
			};

			let label_width = UnicodeWidthStr::width(label) as u16;

			let space_area = Rect { x: inner.x + label_width, y: inner.y + selected as u16, width: 1, height: 1 };

			frame.render_widget(Paragraph::new(" ").style(normal), space_area);

			let field_area = Rect {
				x: inner.x + label_width + 1,
				y: inner.y + selected as u16,
				width: inner.width.saturating_sub(label_width + 1),
				height: 1,
			};
			frame.render_widget(Clear, field_area);
			textarea.render(field_area, frame.buffer_mut());
		}
	}

	// Notes label
	let notes_label_style = if selected == NOTES_INDEX { editing_style } else { label_style };

	let notes_label = Line::from(vec![Span::styled("Notes", notes_label_style), Span::styled(":", normal)]);

	frame.render_widget(Paragraph::new(notes_label), notes_label_area);

	if editing_field && selected == NOTES_INDEX {
		if let Some(textarea) = app.notes_textarea.as_mut() {
			frame.render_widget(Clear, notes_area);
			textarea.render(notes_area, frame.buffer_mut());
		}
	} else {
		frame.render_widget(Paragraph::new(notes_lines), notes_area);
	}

	let help = if let Some(status) = &app.status {
		Paragraph::new(format!("  {status}")).style(Style::new().fg(app.theme.warning))
	} else {
		let help_text = help_lines.iter().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n");

		Paragraph::new(help_text).style(Style::new().fg(app.theme.accent))
	};

	frame.render_widget(help, vertical[1]);
}

#[cfg(test)]
mod tests {
	use super::wrap_notes;

	#[test]
	fn wraps_multiline_notes_to_the_available_width() {
		assert_eq!(wrap_notes("first line\nsecond line with more text", 12), ["first line", "second line", "with more", "text"]);
	}

	#[test]
	fn wraps_long_words_instead_of_clipping_them() {
		assert_eq!(wrap_notes("averylongword", 5), ["avery", "longw", "ord"]);
	}

	#[test]
	fn preserves_blank_lines_in_notes() {
		assert_eq!(wrap_notes("first\n\nthird", 20), ["first", "", "third"]);
	}
}
