//! Generic editable field state and keyboard handling.
//!
//! This module deliberately contains no application or Ratatui UI logic.
//! It owns the text, cursor, editing mode, and viewport calculations used by
//! editable fields in the application.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use zeroize::Zeroize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAction {
	/// The key was handled and editing should continue.
	Continue,
	/// The user wants to accept the current field contents.
	Accept,
	/// The user wants to cancel editing.
	Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldMode {
	/// A field that must remain on one terminal line.
	SingleLine,
	/// A field that may contain newlines.
	Multiline,
}

/// Editable text field.
///
/// The cursor is stored as a byte offset into `text` and is always kept on
/// a UTF-8 character boundary.
///
/// There is deliberately no undo/redo history, clipboard buffer, search
/// state, or other secondary copy of the text.
#[derive(Debug, Clone)]
pub struct FieldEditor {
	text: String,
	cursor: usize,
	preferred_column: Option<usize>,
	mode: FieldMode,
}

impl Drop for FieldEditor {
	fn drop(&mut self) {
		self.text.zeroize();
	}
}

impl Default for FieldEditor {
	fn default() -> Self {
		Self { text: String::new(), cursor: 0, preferred_column: None, mode: FieldMode::SingleLine }
	}
}

impl FieldEditor {
	/// Creates a single-line editor containing `text`, with the cursor at
	/// the beginning of the field.
	pub fn with_text(text: impl Into<String>) -> Self {
		Self::with_mode(text, FieldMode::SingleLine)
	}

	/// Creates a multiline editor containing `text`, with the cursor at the
	/// beginning of the field.
	pub fn with_multiline_text(text: impl Into<String>) -> Self {
		Self::with_mode(text, FieldMode::Multiline)
	}

	/// Creates an editor with an explicitly selected mode.
	pub fn with_mode(text: impl Into<String>, mode: FieldMode) -> Self {
		let text = text.into();
		Self { text, cursor: 0, preferred_column: None, mode }
	}

	pub fn text(&self) -> &str {
		&self.text
	}

	pub fn into_text(mut self) -> String {
		std::mem::take(&mut self.text)
	}

	pub fn cursor(&self) -> usize {
		self.cursor
	}

	pub fn is_single_line(&self) -> bool {
		self.mode == FieldMode::SingleLine
	}

	pub fn is_multiline(&self) -> bool {
		self.mode == FieldMode::Multiline
	}

	pub fn insert(&mut self, ch: char) {
		if self.is_single_line() && ch == '\n' {
			return;
		}

		self.text.insert(self.cursor, ch);
		self.cursor += ch.len_utf8();
		self.preferred_column = None;
	}

	pub fn handle_key(&mut self, key: KeyEvent) -> EditAction {
		match key.code {
			KeyCode::Esc => EditAction::Cancel,

			KeyCode::Enter => {
				if self.is_multiline() {
					self.insert('\n');
					EditAction::Continue
				} else {
					EditAction::Accept
				}
			}

			KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => EditAction::Accept,

			KeyCode::Char(ch) => {
				self.insert(ch);
				EditAction::Continue
			}

			KeyCode::Backspace => {
				self.backspace();
				EditAction::Continue
			}

			KeyCode::Delete => {
				self.delete();
				EditAction::Continue
			}

			KeyCode::Left => {
				self.move_left();
				EditAction::Continue
			}

			KeyCode::Right => {
				self.move_right();
				EditAction::Continue
			}

			KeyCode::Up => {
				self.move_up();
				EditAction::Continue
			}

			KeyCode::Down => {
				self.move_down();
				EditAction::Continue
			}

			KeyCode::Home => {
				self.home();
				EditAction::Continue
			}

			KeyCode::End => {
				self.end();
				EditAction::Continue
			}

			_ => EditAction::Continue,
		}
	}

	pub fn backspace(&mut self) {
		if self.cursor == 0 {
			return;
		}

		let start = previous_char_boundary(&self.text, self.cursor);
		self.text.drain(start..self.cursor);
		self.cursor = start;
		self.preferred_column = None;
	}

	pub fn delete(&mut self) {
		if self.cursor == self.text.len() {
			return;
		}

		let end = next_char_boundary(&self.text, self.cursor);
		self.text.drain(self.cursor..end);
		self.preferred_column = None;
	}

	pub fn move_left(&mut self) {
		if self.cursor == 0 {
			return;
		}

		self.cursor = previous_char_boundary(&self.text, self.cursor);
		self.preferred_column = None;
	}

	pub fn move_right(&mut self) {
		if self.cursor == self.text.len() {
			return;
		}

		self.cursor = next_char_boundary(&self.text, self.cursor);
		self.preferred_column = None;
	}

	pub fn home(&mut self) {
		if self.is_single_line() {
			self.cursor = 0;
		} else {
			self.cursor = line_start(&self.text, self.cursor);
		}

		self.preferred_column = None;
	}

	pub fn end(&mut self) {
		if self.is_single_line() {
			self.cursor = self.text.len();
		} else {
			self.cursor = line_end(&self.text, self.cursor);
		}

		self.preferred_column = None;
	}

	pub fn move_up(&mut self) -> bool {
		if self.is_single_line() {
			return false;
		}

		self.move_vertical(-1)
	}

	pub fn move_down(&mut self) -> bool {
		if self.is_single_line() {
			return false;
		}

		self.move_vertical(1)
	}

	/// Returns the horizontal scroll offset, measured in terminal columns,
	/// needed to keep the cursor visible inside a single-line field.
	///
	/// `width` is the number of terminal columns available for the field.
	///
	/// The returned value is zero for multiline fields and for fields that
	/// fit completely inside the available width.
	pub fn horizontal_scroll(&self, width: usize) -> usize {
		if !self.is_single_line() || width == 0 {
			return 0;
		}

		let cursor_column = self.cursor_display_column();

		if cursor_column < width {
			return 0;
		}

		cursor_column - width + 1
	}

	pub fn single_line_view(&self, width: usize) -> (String, usize) {
		if width == 0 {
			return (String::new(), 0);
		}

		let scroll = self.horizontal_scroll(width);

		let mut start_byte = 0;
		let mut start_column = 0;

		for (byte, ch) in self.text.char_indices() {
			if start_column >= scroll {
				start_byte = byte;
				break;
			}

			start_column += UnicodeWidthChar::width(ch).unwrap_or(0);
			start_byte = byte + ch.len_utf8();
		}

		let mut visible = String::new();
		let mut visible_width = 0;

		for ch in self.text[start_byte..].chars() {
			let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);

			if visible_width + ch_width > width {
				break;
			}

			visible.push(ch);
			visible_width += ch_width;
		}

		let cursor_column = self.cursor_display_column().saturating_sub(start_column);

		(visible, cursor_column.min(width))
	}

	/// Returns the display-column position of the cursor.
	///
	/// This uses Unicode terminal width rather than character count, so
	/// wide characters such as Chinese characters occupy two columns.
	pub fn cursor_display_column(&self) -> usize {
		UnicodeWidthStr::width(&self.text[..self.cursor])
	}

	fn move_vertical(&mut self, direction: i32) -> bool {
		let current_start = line_start(&self.text, self.cursor);
		let current_end = line_end(&self.text, self.cursor);

		let current_column = self.preferred_column.unwrap_or_else(|| self.text[current_start..self.cursor].chars().count());

		let target_start = if direction < 0 {
			if current_start == 0 {
				return false;
			}

			line_start(&self.text, current_start - 1)
		} else {
			if current_end == self.text.len() {
				return false;
			}

			current_end + 1
		};

		let target_end = line_end(&self.text, target_start);
		let target_line = &self.text[target_start..target_end];

		let target_column = target_line.chars().take(current_column).map(char::len_utf8).sum::<usize>();

		self.cursor = target_start + target_column;
		self.preferred_column = Some(current_column);

		true
	}
}

fn previous_char_boundary(text: &str, cursor: usize) -> usize {
	debug_assert!(cursor <= text.len());
	debug_assert!(text.is_char_boundary(cursor));

	if cursor == 0 {
		return 0;
	}

	let mut index = cursor - 1;

	while index > 0 && !text.is_char_boundary(index) {
		index -= 1;
	}

	index
}

fn next_char_boundary(text: &str, cursor: usize) -> usize {
	debug_assert!(cursor <= text.len());
	debug_assert!(text.is_char_boundary(cursor));

	if cursor == text.len() {
		return cursor;
	}

	let mut index = cursor + 1;

	while index < text.len() && !text.is_char_boundary(index) {
		index += 1;
	}

	index
}

fn line_start(text: &str, cursor: usize) -> usize {
	debug_assert!(cursor <= text.len());
	debug_assert!(text.is_char_boundary(cursor));

	match text[..cursor].rfind('\n') {
		Some(index) => index + 1,
		None => 0,
	}
}

fn line_end(text: &str, cursor: usize) -> usize {
	debug_assert!(cursor <= text.len());
	debug_assert!(text.is_char_boundary(cursor));

	match text[cursor..].find('\n') {
		Some(index) => cursor + index,
		None => text.len(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn starts_empty() {
		let editor = FieldEditor::with_text("");

		assert_eq!(editor.text(), "");
		assert_eq!(editor.cursor(), 0);
		assert!(editor.is_single_line());
	}

	#[test]
	fn starts_cursor_at_beginning() {
		let editor = FieldEditor::with_text("abc");

		assert_eq!(editor.cursor(), 0);
	}

	#[test]
	fn inserts_characters() {
		let mut editor = FieldEditor::with_text("");

		editor.insert('a');
		editor.insert('b');
		editor.insert('c');

		assert_eq!(editor.text(), "abc");
		assert_eq!(editor.cursor(), 3);
	}

	#[test]
	fn inserts_unicode() {
		let mut editor = FieldEditor::with_text("");

		editor.insert('日');
		editor.insert('本');

		assert_eq!(editor.text(), "日本");
		assert_eq!(editor.cursor(), 6);

		editor.move_left();

		assert_eq!(editor.cursor(), 3);

		editor.backspace();

		assert_eq!(editor.text(), "本");
		assert_eq!(editor.cursor(), 0);
	}

	#[test]
	fn delete_deletes_next_character() {
		let mut editor = FieldEditor::with_text("abc");

		editor.delete();

		assert_eq!(editor.text(), "bc");
		assert_eq!(editor.cursor(), 0);
	}

	#[test]
	fn home_and_end_cover_entire_single_line_field() {
		let mut editor = FieldEditor::with_text("one");

		editor.end();
		assert_eq!(editor.cursor(), 3);

		editor.home();
		assert_eq!(editor.cursor(), 0);
	}

	#[test]
	fn vertical_movement_preserves_column() {
		let mut editor = FieldEditor::with_multiline_text("abcd\nxy\nabcdef");

		editor.home();
		editor.move_right();
		editor.move_right();
		editor.move_right();
		editor.move_down();
		editor.move_down();

		assert_eq!(editor.cursor(), 11);
	}

	#[test]
	fn vertical_movement_handles_first_and_last_line() {
		let mut editor = FieldEditor::with_multiline_text("one\ntwo");

		assert!(editor.move_down());
		assert!(!editor.move_down());

		editor.end();

		assert!(editor.move_up());
		assert!(!editor.move_up());
	}

	#[test]
	fn single_line_vertical_movement_is_ignored() {
		let mut editor = FieldEditor::with_text("abc");

		editor.end();

		assert!(!editor.move_up());
		assert!(!editor.move_down());
		assert_eq!(editor.cursor(), 3);
	}

	#[test]
	fn ctrl_n_is_ignored() {
		let mut editor = FieldEditor::with_text("abc");

		let action = editor.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));

		assert_eq!(action, EditAction::Continue);
		assert_eq!(editor.text(), "nabc");
		assert_eq!(editor.cursor(), 1);
	}

	#[test]
	fn multiline_enter_inserts_newline() {
		let mut editor = FieldEditor::with_multiline_text("abc");

		editor.end();

		let action = editor.handle_key(KeyEvent::from(KeyCode::Enter));

		assert_eq!(action, EditAction::Continue);
		assert_eq!(editor.text(), "abc\n");
		assert_eq!(editor.cursor(), 4);
	}

	#[test]
	fn multiline_ctrl_s_accepts() {
		let mut editor = FieldEditor::with_multiline_text("abc");

		let action = editor.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

		assert_eq!(action, EditAction::Accept);
		assert_eq!(editor.text(), "abc");
	}

	#[test]
	fn horizontal_scroll_keeps_cursor_visible() {
		let mut editor = FieldEditor::with_text("abcdefghij");

		editor.end();

		assert_eq!(editor.horizontal_scroll(5), 6);
	}

	#[test]
	fn horizontal_scroll_is_zero_when_cursor_fits() {
		let mut editor = FieldEditor::with_text("abc");

		editor.end();

		assert_eq!(editor.horizontal_scroll(5), 0);
	}

	#[test]
	fn horizontal_scroll_accounts_for_wide_characters() {
		let mut editor = FieldEditor::with_text("日本abc");

		editor.end();

		assert_eq!(editor.cursor_display_column(), 7);
		assert_eq!(editor.horizontal_scroll(5), 3);
	}

	#[test]
	fn horizontal_scroll_is_zero_for_multiline_fields() {
		let mut editor = FieldEditor::with_multiline_text("abcdefghij");

		editor.end();

		assert_eq!(editor.horizontal_scroll(5), 0);
	}

	#[test]
	fn handle_key_edits_text() {
		let mut editor = FieldEditor::with_text("");

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Char('a'))), EditAction::Continue);

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Backspace)), EditAction::Continue);

		assert_eq!(editor.text(), "");
	}

	#[test]
	fn handle_key_returns_lifecycle_actions() {
		let mut editor = FieldEditor::with_text("");

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Enter)), EditAction::Accept);

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Esc)), EditAction::Cancel);
	}

	#[test]
	fn single_line_enter_accepts() {
		let mut editor = FieldEditor::with_text("");

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Enter)), EditAction::Accept);
	}
}
