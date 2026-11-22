use ratatui::{
	Frame,
	layout::{Constraint, Layout, Margin, Rect},
	style::Style,
	text::{Line, Span},
	widgets::{Clear, List, ListItem, Paragraph},
};
use std::borrow::Cow;
use std::ops::Range;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, TotpCache};
use crate::db::current_totp_code;
use crate::input::fieldview::{cursor_position, wrap_ranges};
use crate::util::wrap_help_items;

const NAME_INDEX: usize = 0;
const USER_INDEX: usize = 1;
const PASSWORD_INDEX: usize = 2;
const URL_INDEX: usize = 3;
const TOTP_INDEX: usize = 4;
const NOTES_INDEX: usize = 5;

/// The Control-R hint. "Show" and "Hide" have the same width, so the help
/// lines wrap the same either way.
fn reveal_item(slim_mode: bool, show_password: bool) -> &'static str {
	match (slim_mode, show_password) {
		(true, _) => "[^r]",
		(false, false) => "[^r] Show",
		(false, true) => "[^r] Hide",
	}
}

fn nav_help_items(slim_mode: bool, show_password: bool, tmux_copy: bool) -> Vec<&'static str> {
	let reveal = reveal_item(slim_mode, show_password);
	let mut items = if slim_mode {
		vec!["[PgUpDn]", "[Enter]", reveal, "[^s]", "[Esc]"]
	} else {
		vec!["[PgUpDn] Scroll notes", "[Enter] Edit", reveal, "[^s] Save", "[Esc] Close"]
	};
	if tmux_copy {
		items.insert(2, if slim_mode { "[^c]" } else { "[^c] Copy" });
	}
	items
}

/// `password` is set when editing the password: whether it's shown.
fn field_help_items(slim_mode: bool, multiline: bool, password: Option<bool>) -> Vec<&'static str> {
	let mut items = match (slim_mode, multiline) {
		(true, true) => vec!["[^s]", "[Esc]"],
		(true, false) => vec!["[Enter]", "[Esc]"],
		(false, true) => vec!["[^s] Save field", "[Esc] Cancel"],
		(false, false) => vec!["[Enter] Save field", "[Esc] Cancel"],
	};
	if let Some(shown) = password {
		items.insert(1, reveal_item(slim_mode, shown));
	}
	items
}

/// Hides a password behind the same dots as on the login screen. Every
/// character becomes as many dots as it is wide, so the cursor and the visible
/// part of the field editor line up just like with the password itself.
fn masked(text: &str) -> String {
	text.graphemes(true).map(|cluster| crate::DOT.repeat(cluster.width())).collect()
}

fn split_edit_area(area: Rect, help_line_count: usize) -> (Rect, Rect) {
	let help_height = u16::try_from(help_line_count).unwrap_or(u16::MAX);
	let [body, help] = Layout::vertical([Constraint::Fill(1), Constraint::Length(help_height)]).areas(area);
	(body, help)
}

#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
pub(crate) fn field_at_position(area: Rect, slim_mode: bool, tmux_copy: bool, column: f64, row: u16) -> Option<(usize, bool)> {
	// The help wraps the same whether the password is shown (see `reveal_item`).
	let help_lines = wrap_help_items(&nav_help_items(slim_mode, false, tmux_copy), area.width);
	let (body, _) = split_edit_area(area, help_lines.len());
	let inner = body.inner(Margin { horizontal: 0, vertical: 1 });
	let field = usize::from(row.checked_sub(inner.y)?).min(NOTES_INDEX);
	if row >= body.bottom() || !(f64::from(inner.x)..f64::from(inner.right())).contains(&column) {
		return None;
	}
	let label_end = if field == NOTES_INDEX { inner.x.saturating_add("Notes:".width() as u16) } else { value_area(inner, field).x };
	let on_label = row == inner.y.saturating_add(field as u16) && column < f64::from(label_end);
	Some((field, on_label))
}

/// Geometry of the text currently visible in the editor. These byte ranges
/// refer to its existing buffer and never contain copies of field contents.
pub(crate) struct EditView {
	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	area: Rect,
	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	rows: Vec<Range<usize>>,
	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	pub(crate) highlights: Vec<Rect>,
}

impl EditView {
	pub(super) fn new(area: Rect, rows: impl Iterator<Item = Range<usize>>, text: &str, selection: Option<&Range<usize>>) -> Self {
		#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
		{
			let rows: Vec<_> = rows.collect();
			let highlights = selection.map_or_else(Vec::new, |selection| {
				rows.iter()
					.enumerate()
					.filter_map(|(row, visible)| selection_rect(area, row as u16, text, visible, selection))
					.collect()
			});
			Self { area, rows, highlights }
		}
		#[cfg(not(all(feature = "gui", target_os = "linux", target_env = "gnu")))]
		{
			let _ = (area, rows, text, selection);
			Self {}
		}
	}

	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	pub(crate) fn line_at(&self, row: f64) -> Option<&Range<usize>> {
		let row = row - f64::from(self.area.y);
		if !(0.0..f64::from(self.area.height)).contains(&row) {
			return None;
		}
		self.rows.get(row as usize)
	}

	#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
	pub(crate) fn offset_at(&self, text: &str, column: f64, row: f64, dragging: bool) -> Option<usize> {
		if self.area.is_empty() {
			return None;
		}
		let x = column - f64::from(self.area.x);
		let y = row - f64::from(self.area.y);
		if !dragging && (!(0.0..f64::from(self.area.width)).contains(&x) || !(0.0..f64::from(self.area.height)).contains(&y)) {
			return None;
		}
		let Some(range) = self.rows.get(y.max(0.0) as usize) else {
			return self.rows.last().map(|range| range.end);
		};
		let line = text.get(range.clone())?;
		let mut left = 0.0;
		for (byte, cluster) in line.grapheme_indices(true) {
			let width = cluster.width() as f64;
			if x < left + width / 2.0 {
				return Some(range.start + byte);
			}
			left += width;
		}
		Some(range.end)
	}
}

#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
fn selection_rect(area: Rect, row: u16, text: &str, visible: &Range<usize>, selection: &Range<usize>) -> Option<Rect> {
	let start = visible.start.max(selection.start);
	let end = visible.end.min(selection.end);
	let includes_newline = selection.contains(&visible.end) && matches!(text.as_bytes().get(visible.end), Some(b'\n' | b'\r'));
	if start > end || (start == end && !includes_newline) {
		return None;
	}

	let col = text.get(visible.start..start)?.width();
	// A selected newline fills through the end of its displayed row.
	let right = if includes_newline {
		usize::from(area.width)
	} else {
		text.get(visible.start..end)?.width().min(usize::from(area.width))
	};
	(col < right).then(|| Rect::new(area.x + col as u16, area.y + row, (right - col) as u16, 1))
}

fn value_area(inner: Rect, field: usize) -> Rect {
	let label = match field {
		NAME_INDEX => "Name: ",
		USER_INDEX => "User: ",
		PASSWORD_INDEX => "Password: ",
		URL_INDEX => "URL: ",
		TOTP_INDEX => "TOTP: ",
		_ => "",
	};
	let offset = label.width() as u16;
	Rect {
		x: inner.x.saturating_add(offset),
		y: inner.y.saturating_add(field as u16),
		width: inner.width.saturating_sub(offset),
		height: 1,
	}
}

fn notes_area(body: Rect, inner: Rect) -> Rect {
	let y = inner.y.saturating_add(NOTES_INDEX as u16 + 1);
	Rect { x: inner.x, y, width: inner.width, height: body.bottom().saturating_sub(y) }
}

#[cfg(all(feature = "gui", target_os = "linux", target_env = "gnu"))]
pub(crate) fn readonly_view<'a>(area: Rect, app: &'a App, field: usize, selection: Option<&Range<usize>>) -> Option<(EditView, &'a str)> {
	let entry = app.edit_entry.as_ref()?;
	let text = match field {
		NAME_INDEX => entry.name.as_str(),
		USER_INDEX => entry.user.as_str(),
		// Also when hidden: the dots line up with it, so it can be copied without showing it.
		PASSWORD_INDEX => entry.password.as_str(),
		URL_INDEX => entry.url.as_str(),
		TOTP_INDEX => app.totp_cache.as_ref()?.code.as_str(),
		NOTES_INDEX => entry.notes.as_str(),
		_ => return None,
	};
	if text.is_empty() {
		return None;
	}

	let help_lines = wrap_help_items(&nav_help_items(app.slim_mode, app.show_password, app.tmux_copy()), area.width);
	let (body, _) = split_edit_area(area, help_lines.len());
	let inner = body.inner(Margin { horizontal: 0, vertical: 1 });
	let view = if field == NOTES_INDEX {
		let area = notes_area(body, inner);
		let ranges = wrap_ranges(text, area.width.saturating_sub(1) as usize);
		let height = usize::from(area.height);
		let scroll = app.field_editor_scroll.min(ranges.len().saturating_sub(height));
		EditView::new(area, ranges.into_iter().skip(scroll).take(height), text, selection)
	} else {
		let area = value_area(inner, field);
		let mut width = 0;
		let mut end = 0;
		for cluster in text.graphemes(true) {
			width += cluster.width();
			if width > usize::from(area.width) || cluster.chars().any(char::is_control) {
				break;
			}
			end += cluster.len();
		}
		EditView::new(area, std::iter::once(0..end), text, selection)
	};
	Some((view, text))
}

fn compact_field<'a>(
	selected: usize,
	field_index: usize,
	label: &'static str,
	value: &'a str,
	extra: Vec<Span<'a>>,
	show_empty: bool,
	styles: [Style; 4], // [label, editing, normal, placeholder]
) -> ListItem<'a> {
	let [label_style, editing_style, normal, placeholder] = styles;
	let label_style = if selected == field_index { editing_style } else { label_style };
	let mut line = vec![Span::styled(label, label_style), Span::styled(": ", normal)];
	if value.is_empty() && extra.is_empty() {
		if show_empty {
			line.push(Span::styled("(empty)", placeholder));
		}
	} else {
		line.push(Span::styled(value, normal));
		line.extend(extra);
	}
	ListItem::new(Line::from(line))
}

pub fn draw_edit(frame: &mut Frame, app: &mut App) -> Option<EditView> {
	let full_area = frame.area();
	let entry = app.edit_entry.as_ref()?;
	let mut edit_view = None;

	let name = entry.name.as_str();
	let user = entry.user.as_str();
	let url = entry.url.as_str();
	let totp_raw = entry.totp.as_str();
	let notes = entry.notes.as_str();
	let last_modified = entry.date_last_modify.as_str();
	let password = entry.password.as_str();
	let password_is_set = !password.is_empty();
	let is_new_entry = app.edit_target.is_none();
	let editing_field = app.editing_field;
	let selected = app.edit_state.selected().unwrap_or(0);
	let totp_active = !(editing_field && selected == TOTP_INDEX);
	if totp_active && app.totp_cache.as_ref().is_none_or(|c| c.expires <= Instant::now()) {
		app.totp_cache = current_totp_code(totp_raw).map(|t| TotpCache { code: t.code, next: t.next, expires: Instant::now() + t.valid_for });
	}
	let totp_code: Option<(&str, &str, Duration)> = if totp_active {
		app.totp_cache.as_ref().map(|c| (c.code.as_str(), c.next.as_str(), c.expires.saturating_duration_since(Instant::now())))
	} else {
		None
	};
	let theme = &app.theme;
	let label_style = Style::new().fg(theme.header).bold();
	let normal = Style::new().fg(theme.text);
	let warning = Style::new().fg(theme.warning);
	let placeholder = warning;
	let editing_style = Style::new().fg(theme.selection_fg).bg(theme.selection_bg);
	let accent_style = Style::new().fg(theme.selection_fg).bg(theme.accent);
	let help_items = if editing_field {
		field_help_items(app.slim_mode, selected == NOTES_INDEX, (selected == PASSWORD_INDEX).then_some(app.show_password))
	} else {
		nav_help_items(app.slim_mode, app.show_password, app.tmux_copy())
	};
	let help_width = full_area.width;
	let help_lines = wrap_help_items(&help_items, help_width);
	let (list_area, help_area) = split_edit_area(full_area, help_lines.len());
	let inner = list_area.inner(Margin { horizontal: 0, vertical: 1 });
	let notes_label_y = inner.y.saturating_add(NOTES_INDEX as u16);
	let notes_label_area = Rect { x: inner.x, y: notes_label_y, width: inner.width, height: 1 };
	let notes_area = notes_area(list_area, inner);
	let notes_width = notes_area.width.saturating_sub(1) as usize;
	let notes_ranges = if notes.is_empty() { Vec::new() } else { wrap_ranges(notes, notes_width) };
	let notes_lines: Vec<Line> = if notes.is_empty() {
		vec![Line::from(Span::styled("(empty)", placeholder))]
	} else {
		notes_ranges.iter().map(|r| Line::from(Span::styled(&notes[r.clone()], normal))).collect()
	};
	let max_notes_scroll = notes_lines.len().saturating_sub(notes_area.height as usize);
	app.field_editor_scroll = app.field_editor_scroll.min(max_notes_scroll);
	let totp_extra = match totp_code.as_ref() {
		Some(_) => vec![],
		None if totp_raw.is_empty() || (editing_field && selected == TOTP_INDEX) => {
			vec![]
		}
		None => {
			vec![Span::styled("couldn't generate a code", warning)]
		}
	};
	let styles = [label_style, editing_style, normal, placeholder];
	let items = vec![
		compact_field(selected, NAME_INDEX, "Name", name, vec![], true, styles),
		compact_field(selected, USER_INDEX, "User", user, vec![], true, styles),
		compact_field(selected, PASSWORD_INDEX, "Password", "", vec![], !password_is_set, styles),
		compact_field(selected, URL_INDEX, "URL", url, vec![], true, styles),
		compact_field(selected, TOTP_INDEX, "TOTP", "", totp_extra, totp_raw.is_empty(), styles),
	];
	let place = if app.groups.is_empty() { String::new() } else { format!(" in {}", app.group_path(entry.group)) };
	let title = if is_new_entry {
		format!("New Entry{place} ")
	} else if last_modified.is_empty() {
		format!("Entry{place} ")
	} else {
		format!("Entry{place} - modified {last_modified} ")
	};
	let list = List::new(items).block(super::section(&title, theme, 0));
	let mut list_state = app.edit_state;
	if selected == NOTES_INDEX {
		list_state.select(None);
	} else {
		list_state.select(Some(selected));
	}
	frame.render_stateful_widget(list, list_area, &mut list_state);

	// Render the password separately from the List, hidden unless shown with Control-R.
	if password_is_set {
		let field_area = value_area(inner, PASSWORD_INDEX);
		let shown = if app.show_password { Cow::Borrowed(password) } else { Cow::Owned(masked(password)) };
		frame.render_widget(Paragraph::new(shown.as_ref()).style(normal), field_area);
	}

	// Render the current TOTP code separately from the List.
	// The otpauth URI/secret is deliberately not rendered while viewing.
	if let Some((code, next, valid_for)) = totp_code.as_ref() {
		let field_area = value_area(inner, TOTP_INDEX);
		let line = Line::from(vec![
			Span::styled(*code, normal.bold()),
			Span::styled(format!("  {:>2}   (", valid_for.as_secs()), warning),
			Span::styled(*next, warning),
			Span::styled(")", warning),
		]);
		frame.render_widget(Paragraph::new(line), field_area);
	}

	// Overlay the single-line field editor when editing
	// Name/User/Password/URL/TOTP.
	if editing_field
		&& selected <= TOTP_INDEX
		&& let Some(editor) = app.field_editor.as_ref()
	{
		let field_area = value_area(inner, selected);
		frame.render_widget(Clear, field_area);

		let available_width = field_area.width as usize;
		let (visible, cursor_column) = editor.single_line_range(available_width);
		let text = editor.text();
		let hide = selected == PASSWORD_INDEX && !app.show_password;
		let shown = if hide { Cow::Owned(masked(&text[visible.clone()])) } else { Cow::Borrowed(&text[visible.clone()]) };
		frame.render_widget(Paragraph::new(shown.as_ref()).style(super::input_style(theme)), field_area);
		let selection = editor.selection();
		edit_view = Some(EditView::new(field_area, std::iter::once(visible), text, selection.as_ref()));

		let cursor_x = field_area.x.saturating_add(u16::try_from(cursor_column).unwrap_or(u16::MAX));
		if selection.is_none()
			&& cursor_column < available_width
			&& let Some(cell) = frame.buffer_mut().cell_mut((cursor_x, field_area.y))
		{
			cell.set_style(accent_style);
		}
	}
	let notes_label_style = if selected == NOTES_INDEX { editing_style } else { label_style };
	let notes_label = Line::from(vec![Span::styled("Notes", notes_label_style), Span::styled(":", normal)]);
	frame.render_widget(Paragraph::new(notes_label), notes_label_area);

	if editing_field && selected == NOTES_INDEX {
		if let Some(editor) = app.field_editor.as_ref() {
			frame.render_widget(Clear, notes_area);
			let text = editor.text();
			let ranges = wrap_ranges(text, notes_width);
			let (cursor_row, cursor_col) = cursor_position(text, &ranges, editor.cursor());
			let visible_height = notes_area.height as usize;
			if cursor_row < app.field_editor_scroll {
				app.field_editor_scroll = cursor_row;
			} else if cursor_row >= app.field_editor_scroll.saturating_add(visible_height) {
				app.field_editor_scroll = cursor_row - visible_height + 1;
			}
			let visible_lines = ranges
				.iter()
				.skip(app.field_editor_scroll)
				.take(visible_height)
				.map(|r| Line::from(Span::styled(&text[r.clone()], normal)))
				.collect::<Vec<_>>();
			frame.render_widget(Paragraph::new(visible_lines).style(super::input_style(theme)), notes_area);
			let visible = ranges.into_iter().skip(app.field_editor_scroll).take(visible_height);
			let selection = editor.selection();
			edit_view = Some(EditView::new(notes_area, visible, text, selection.as_ref()));
			let visible_row = cursor_row.saturating_sub(app.field_editor_scroll);
			if selection.is_none() && cursor_col < notes_area.width as usize && visible_row < notes_area.height as usize {
				let x = notes_area.x.saturating_add(u16::try_from(cursor_col).unwrap_or(u16::MAX));
				let y = notes_area.y.saturating_add(u16::try_from(visible_row).unwrap_or(u16::MAX));
				if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
					cell.set_style(accent_style);
				}
			}
		}
	} else {
		let visible_lines = notes_lines.into_iter().skip(app.field_editor_scroll).take(notes_area.height as usize).collect::<Vec<_>>();
		frame.render_widget(Paragraph::new(visible_lines), notes_area);
	}
	let help = if let Some(status) = &app.status {
		Paragraph::new(status.as_str()).style(Style::new().fg(app.theme.warning))
	} else {
		let help_text = help_lines.join("\n");
		Paragraph::new(help_text).style(Style::new().fg(app.theme.accent))
	};
	frame.render_widget(help, help_area);
	edit_view
}

#[cfg(test)]
mod mask_tests {
	use super::{masked, nav_help_items, reveal_item};
	use unicode_width::UnicodeWidthStr;

	#[test]
	fn masks_keep_the_width_of_the_password() {
		assert_eq!(masked(""), "");
		assert_eq!(masked("pässwörd"), crate::DOT.repeat(8));
		assert_eq!(masked("猫z"), crate::DOT.repeat(3));
		for text in ["correct horse", "猫z", "e\u{301}x"] {
			assert_eq!(masked(text).width(), text.width(), "{text}");
		}
	}

	#[test]
	fn the_reveal_hint_has_the_same_width_shown_or_hidden() {
		for slim in [false, true] {
			assert_eq!(reveal_item(slim, false).width(), reveal_item(slim, true).width());
			assert_eq!(nav_help_items(slim, false, false).concat().width(), nav_help_items(slim, true, false).concat().width());
		}
	}
}

#[cfg(all(test, feature = "gui", target_os = "linux", target_env = "gnu"))]
mod tests {
	use super::*;

	#[test]
	fn a_hidden_password_can_be_selected() {
		let mut app = crate::app::start(false, zeroize::Zeroizing::new(String::new()));
		let mut entry = crate::db::Entry::default();
		entry.password = "pä猫z".to_string();
		app.edit_entry = Some(entry);
		assert!(!app.show_password);
		let (_, text) = readonly_view(Rect::new(0, 0, 96, 26), &app, PASSWORD_INDEX, None).expect("password should be selectable");
		assert_eq!(text, "pä猫z");
	}

	#[test]
	fn selecting_a_hidden_password_lines_up_with_its_dots() {
		let text = "pä猫z";
		let selection = "p".len().."pä猫".len();
		let view = EditView::new(Rect::new(10, 3, 20, 1), std::iter::once(0..text.len()), text, Some(&selection));
		let before = masked(&text[..selection.start]).width() as u16;
		let selected = masked(&text[selection.clone()]).width() as u16;
		assert_eq!(view.highlights, [Rect::new(10 + before, 3, selected, 1)]);
		// On either dot of the wide character.
		assert_eq!(view.offset_at(text, 12.2, 3.0, false), Some("pä".len()));
		assert_eq!(view.offset_at(text, 13.5, 3.0, false), Some("pä猫".len()));
	}

	#[test]
	fn mouse_hit_testing_and_highlight_follow_visible_graphemes() {
		let text = "น้ำ
猫z";
		let rows = wrap_ranges(text, 10);
		let area = Rect::new(3, 4, 10, 3);
		let second = "น้ำ
"
		.len();
		let selection = 0..second + "猫".len();
		let view = EditView::new(area, rows.into_iter(), text, Some(&selection));

		assert_eq!(view.offset_at(text, 2.9, 4.0, false), None);
		assert_eq!(view.offset_at(text, 3.4, 4.0, false), Some(0));
		assert_eq!(view.offset_at(text, 4.6, 4.0, false), Some("น้ำ".len()));
		assert_eq!(view.offset_at(text, 4.5, 5.0, true), Some(second + "猫".len()));
		assert_eq!(view.offset_at(text, 3.0, 6.0, true), Some(text.len()));
		assert_eq!(view.highlights, [Rect::new(3, 4, 10, 1), Rect::new(3, 5, 2, 1)]);
	}
}

#[cfg(test)]
mod totp_display_tests {
	use super::*;
	use ratatui::{Terminal, backend::TestBackend};
	use zeroize::Zeroizing;

	#[test]
	fn renders_zero_seconds_and_the_next_code() {
		let mut app = crate::app::start(false, Zeroizing::new(String::new()));
		let mut entry = crate::db::Entry::default();
		entry.name = "TOTP display test".to_string();
		entry.totp = "configured".to_string();
		app.edit_entry = Some(entry);
		app.totp_cache = Some(TotpCache {
			code: Zeroizing::new("123456".to_string()),
			next: Zeroizing::new("098765".to_string()),
			expires: Instant::now() + Duration::from_millis(900),
		});
		let mut terminal = Terminal::new(TestBackend::new(96, 26)).unwrap();
		let frame = terminal
			.draw(|frame| {
				draw_edit(frame, &mut app);
			})
			.unwrap();
		let rendered: String = frame.buffer.content.iter().map(|cell| cell.symbol()).collect();
		assert!(rendered.contains("TOTP: 123456   0   (098765)"));
	}
}
