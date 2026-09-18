//! Generic editable field state and keyboard handling.
//!
//! This module deliberately contains no application or Ratatui UI logic.
//! It owns the text, cursor, and basic editing operations used by editable
//! fields in the application.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

/// Editable text field.
///
/// The cursor is stored as a byte offset into `text` and is always kept on
/// a UTF-8 character boundary.
///
/// There is deliberately no undo/redo history, clipboard buffer, search
/// state, or other secondary copy of the text.
#[derive(Debug, Clone, Default)]
pub struct FieldEditor {
	text: String,
	cursor: usize,
	preferred_column: Option<usize>,
}

impl Drop for FieldEditor {
	fn drop(&mut self) {
		self.text.zeroize();
	}
}

impl FieldEditor {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn with_text(text: impl Into<String>) -> Self {
		let text = text.into();

		Self { cursor: 0, text, preferred_column: None }
	}

	pub fn text(&self) -> &str {
		&self.text
	}

	pub fn set_text(&mut self, text: impl Into<String>) {
		self.text = text.into();
		self.cursor = self.text.len();
		self.preferred_column = None;
	}

	pub fn cursor(&self) -> usize {
		self.cursor
	}

	pub fn is_empty(&self) -> bool {
		self.text.is_empty()
	}

	pub fn len(&self) -> usize {
		self.text.len()
	}

	pub fn insert(&mut self, ch: char) {
		self.text.insert(self.cursor, ch);
		self.cursor += ch.len_utf8();
		self.preferred_column = None;
	}

	pub fn insert_str(&mut self, text: &str) {
		self.text.insert_str(self.cursor, text);
		self.cursor += text.len();
		self.preferred_column = None;
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
		self.cursor = line_start(&self.text, self.cursor);
		self.preferred_column = None;
	}

	pub fn end(&mut self) {
		self.cursor = line_end(&self.text, self.cursor);
		self.preferred_column = None;
	}

	pub fn move_up(&mut self) -> bool {
		self.move_vertical(-1)
	}

	pub fn move_down(&mut self) -> bool {
		self.move_vertical(1)
	}

	pub fn clear(&mut self) {
		self.text.clear();
		self.cursor = 0;
		self.preferred_column = None;
	}

	/// Handles a key that is intended for the field editor.
	///
	/// Enter and Escape are returned to the caller as actions rather than
	/// modifying the field. This lets settings/edit decide what accepting
	/// or cancelling means for their respective screens.
	pub fn handle_key(&mut self, key: KeyEvent) -> EditAction {
		match key.code {
			KeyCode::Esc => EditAction::Cancel,

			KeyCode::Enter => EditAction::Accept,

			KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
				self.insert('\n');
				EditAction::Continue
			}

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

	pub fn current_line_range(&self) -> std::ops::Range<usize> {
		line_start(&self.text, self.cursor)..line_end(&self.text, self.cursor)
	}

	pub fn current_line(&self) -> &str {
		let range = self.current_line_range();
		&self.text[range]
	}

	pub fn line(&self) -> usize {
		self.text[..self.cursor].bytes().filter(|&byte| byte == b'\n').count()
	}

	pub fn column(&self) -> usize {
		let start = line_start(&self.text, self.cursor);
		self.text[start..self.cursor].chars().count()
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
		let editor = FieldEditor::new();

		assert_eq!(editor.text(), "");
		assert_eq!(editor.cursor(), 0);
		assert!(editor.is_empty());
	}

	#[test]
	fn inserts_characters() {
		let mut editor = FieldEditor::new();

		editor.insert('a');
		editor.insert('b');
		editor.insert('c');

		assert_eq!(editor.text(), "abc");
		assert_eq!(editor.cursor(), 3);
	}

	#[test]
	fn inserts_unicode() {
		let mut editor = FieldEditor::new();

		editor.insert('日');
		editor.insert('本');

		assert_eq!(editor.text(), "日本");
		assert_eq!(editor.cursor(), 6);

		editor.move_left();

		assert_eq!(editor.cursor(), 3);

		editor.backspace();

		assert_eq!(editor.text(), "日");
		assert_eq!(editor.cursor(), 0);
	}

	#[test]
	fn backspace_deletes_previous_character() {
		let mut editor = FieldEditor::with_text("abc");

		editor.backspace();

		assert_eq!(editor.text(), "ab");
		assert_eq!(editor.cursor(), 2);
	}

	#[test]
	fn delete_deletes_next_character() {
		let mut editor = FieldEditor::with_text("abc");

		editor.home();
		editor.delete();

		assert_eq!(editor.text(), "bc");
		assert_eq!(editor.cursor(), 0);
	}

	#[test]
	fn home_and_end_stay_on_current_line() {
		let mut editor = FieldEditor::with_text("one\ntwo\nthree");

		editor.home();

		assert_eq!(editor.cursor(), 0);

		editor.move_down();
		editor.end();

		assert_eq!(editor.current_line(), "two");
		assert_eq!(editor.cursor(), 7);
	}

	#[test]
	fn vertical_movement_preserves_column() {
		let mut editor = FieldEditor::with_text("abcd\nxy\nabcdef");

		editor.home();
		editor.move_right();
		editor.move_right();
		editor.move_right();

		assert_eq!(editor.column(), 3);

		editor.move_down();

		assert_eq!(editor.current_line(), "xy");
		assert_eq!(editor.column(), 2);

		editor.move_down();

		assert_eq!(editor.current_line(), "abcdef");
		assert_eq!(editor.column(), 3);
	}

	#[test]
	fn vertical_movement_handles_first_and_last_line() {
		let mut editor = FieldEditor::with_text("one\ntwo");

		assert!(!editor.move_up());

		editor.end();

		assert!(!editor.move_down());
	}

	#[test]
	fn current_line_range_excludes_newline() {
		let editor = FieldEditor::with_text("one\ntwo\nthree");

		assert_eq!(&editor.text()[editor.current_line_range()], "three");
	}

	#[test]
	fn clear_removes_text() {
		let mut editor = FieldEditor::with_text("secret");

		editor.clear();

		assert!(editor.is_empty());
		assert_eq!(editor.cursor(), 0);
	}

	#[test]
	fn handle_key_edits_text() {
		let mut editor = FieldEditor::new();

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Char('a'))), EditAction::Continue);

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Backspace)), EditAction::Continue);

		assert_eq!(editor.text(), "");
	}

	#[test]
	fn handle_key_returns_lifecycle_actions() {
		let mut editor = FieldEditor::new();

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Enter)), EditAction::Accept);

		assert_eq!(editor.handle_key(KeyEvent::from(KeyCode::Esc)), EditAction::Cancel);
	}
}
