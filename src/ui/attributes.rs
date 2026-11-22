//! The view of the additional attributes of the open entry (see `input::attributes`).

use std::borrow::Cow;

use ratatui::{
	Frame,
	layout::{Constraint, Layout, Rect},
	style::Style,
	text::{Line, Span},
	widgets::{List, ListItem, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::edit::EditView;
use crate::app::{App, AttributeEdit};
use crate::input::fieldview::{cursor_position, wrap_ranges};
use crate::util::wrap_help_items;

/// Hides a protected value behind asterisks, as wide as the text.
fn hidden(text: &str) -> String {
	text.graphemes(true).map(|cluster| "*".repeat(cluster.width())).collect()
}

/// Shows a protected value hidden, unless shown with Control-R.
fn shown(text: &str, hide: bool) -> Cow<'_, str> {
	if hide { Cow::Owned(hidden(text)) } else { Cow::Borrowed(text) }
}

fn help_items(slim_mode: bool, edit: Option<AttributeEdit>, protected: bool, show_protected: bool) -> Vec<&'static str> {
	let reveal = match (slim_mode, show_protected) {
		(true, _) => "[^r]",
		(false, false) => "[^r] Show",
		(false, true) => "[^r] Hide",
	};
	match (edit, slim_mode) {
		(Some(AttributeEdit::Name(_)), true) => vec!["[Enter]", "[Esc]"],
		(Some(AttributeEdit::Name(_)), false) => vec!["[Enter] Save name", "[Esc] Cancel"],
		(Some(AttributeEdit::Value(_)), true) if protected => vec!["[^s]", reveal, "[Esc]"],
		(Some(AttributeEdit::Value(_)), true) => vec!["[^s]", "[Esc]"],
		(Some(AttributeEdit::Value(_)), false) if protected => vec!["[^s] Save value", reveal, "[Esc] Cancel"],
		(Some(AttributeEdit::Value(_)), false) => vec!["[^s] Save value", "[Esc] Cancel"],
		(None, true) => vec!["[Enter]", "[^a]", "[^n]", "[^p]", reveal, "[^d]", "[^s]", "[Esc]"],
		(None, false) => vec![
			"[Enter] Edit",
			"[^a] Add",
			"[^n] Rename",
			if protected { "[^p] Unprotect" } else { "[^p] Protect" },
			reveal,
			"[^d] Delete",
			"[^s] Save",
			"[Esc] Back",
		],
	}
}

pub fn draw_attributes(frame: &mut Frame, app: &mut App) -> Option<EditView> {
	let area = frame.area();
	let entry = app.edit_entry.as_ref()?;
	let theme = &app.theme;
	let normal = Style::new().fg(theme.text);
	let label_style = Style::new().fg(theme.header).bold();
	let placeholder = Style::new().fg(theme.text).bold();
	let editing_style = Style::new().fg(theme.selection_fg).bg(theme.selection_bg);
	let accent_style = Style::new().fg(theme.selection_fg).bg(theme.accent);

	let selected = app.attribute_state.selected().and_then(|index| entry.attributes.get(index));
	let protected = selected.is_some_and(|attribute| attribute.protected);
	let help_lines = wrap_help_items(&help_items(app.slim_mode, app.attribute_edit, protected, app.show_protected), area.width);
	let help_height = u16::try_from(help_lines.len()).unwrap_or(u16::MAX);
	let [list_area, value_area, help_area] = Layout::vertical([Constraint::Fill(1), Constraint::Fill(1), Constraint::Length(help_height)]).areas(area);

	// The list: names, and the first line of each value.
	let items: Vec<ListItem> = if entry.attributes.is_empty() {
		vec![ListItem::new(Span::styled("(none)", placeholder))]
	} else {
		entry
			.attributes
			.iter()
			.map(|attribute| {
				let first = attribute.value.lines().next().unwrap_or_default();
				let mut line = vec![
					Span::styled(attribute.name.as_str(), label_style),
					Span::styled(": ", normal),
					Span::styled(shown(first, attribute.protected && !app.show_protected), normal),
				];
				if attribute.value.contains('\n') {
					line.push(Span::styled(" ...", placeholder));
				}
				ListItem::new(Line::from(line))
			})
			.collect()
	};
	let title = format!("Attributes of {} ", if entry.name.is_empty() { "the new entry" } else { entry.name.as_str() });
	let list = List::new(items).block(super::section(&title, theme, 0)).highlight_style(editing_style);
	frame.render_stateful_widget(list, list_area, &mut app.attribute_state);

	// The value of the selected attribute, or the editor.
	let value_title = match (app.attribute_edit, selected) {
		(Some(AttributeEdit::Name(None)), _) => "New attribute name ".to_string(),
		(Some(AttributeEdit::Name(Some(_))), Some(attribute)) => format!("Rename \"{}\" ", attribute.name),
		(_, Some(attribute)) if attribute.protected => format!("{} (protected) ", attribute.name),
		(_, Some(attribute)) => format!("{} ", attribute.name),
		(_, None) => " ".to_string(),
	};
	let section = super::section(&value_title, theme, 0);
	let inner = section.inner(value_area);
	frame.render_widget(section, value_area);
	let width = usize::from(inner.width.saturating_sub(1));
	let height = usize::from(inner.height);

	match (app.attribute_edit, app.field_editor.as_ref()) {
		(Some(AttributeEdit::Name(_)), Some(editor)) => {
			let row = Rect { height: 1.min(inner.height), ..inner };
			let (visible, cursor_column) = editor.single_line_range(usize::from(row.width));
			frame.render_widget(Paragraph::new(&editor.text()[visible]).style(super::input_style(theme)), row);
			if editor.selection().is_none()
				&& let Some(cell) = frame.buffer_mut().cell_mut((row.x.saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX)), row.y))
			{
				cell.set_style(accent_style);
			}
		}
		(Some(AttributeEdit::Value(_)), Some(editor)) => {
			let hide = protected && !app.show_protected;
			let text = editor.text();
			let ranges = wrap_ranges(text, width);
			let (cursor_row, cursor_column) = cursor_position(text, &ranges, editor.cursor());
			if cursor_row < app.attribute_scroll {
				app.attribute_scroll = cursor_row;
			} else if cursor_row >= app.attribute_scroll + height.max(1) {
				app.attribute_scroll = cursor_row + 1 - height.max(1);
			}
			let lines: Vec<Line> = ranges
				.iter()
				.skip(app.attribute_scroll)
				.take(height)
				.map(|range| Line::from(shown(&text[range.clone()], hide)))
				.collect();
			frame.render_widget(Paragraph::new(lines).style(super::input_style(theme)), inner);
			let row = cursor_row - app.attribute_scroll;
			if editor.selection().is_none()
				&& cursor_column < usize::from(inner.width)
				&& row < height
				&& let Some(cell) = frame.buffer_mut().cell_mut((
					inner.x.saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX)),
					inner.y.saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
				)) {
				cell.set_style(accent_style);
			}
		}
		_ => match selected {
			Some(attribute) if !attribute.value.is_empty() => {
				let hide = attribute.protected && !app.show_protected;
				let ranges = wrap_ranges(&attribute.value, width);
				app.attribute_scroll = app.attribute_scroll.min(ranges.len().saturating_sub(height));
				let lines: Vec<Line> = ranges
					.iter()
					.skip(app.attribute_scroll)
					.take(height)
					.map(|range| Line::from(shown(&attribute.value[range.clone()], hide)))
					.collect();
				frame.render_widget(Paragraph::new(lines).style(normal), inner);
			}
			Some(_) => frame.render_widget(Paragraph::new(Span::styled("(empty)", placeholder)), inner),
			None => frame.render_widget(Paragraph::new(Span::styled("[^a] adds an attribute", placeholder)), inner),
		},
	}

	let help = match &app.status {
		Some(status) => Paragraph::new(status.as_str()).style(Style::new().fg(theme.warning)),
		None => Paragraph::new(help_lines.join("\n")).style(Style::new().fg(theme.accent)),
	};
	frame.render_widget(help, help_area);
	// The GUI's mouse selection isn't supported here.
	None
}

#[cfg(test)]
mod tests {
	use super::{help_items, hidden};
	use crate::app::AttributeEdit;
	use unicode_width::UnicodeWidthStr;

	#[test]
	fn hidden_values_keep_their_width() {
		assert_eq!(hidden("1234"), "****");
		assert_eq!(hidden("猫z").width(), "猫z".width());
	}

	#[test]
	fn shows_the_reveal_key_for_protected_values_only_while_editing() {
		assert!(help_items(false, Some(AttributeEdit::Value(0)), true, false).contains(&"[^r] Show"));
		assert!(!help_items(false, Some(AttributeEdit::Value(0)), false, false).contains(&"[^r] Show"));
		assert!(help_items(false, None, true, true).contains(&"[^p] Unprotect"));
		assert!(help_items(false, None, false, true).contains(&"[^r] Hide"));
	}
}
