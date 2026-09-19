use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use zeroize::Zeroizing;

pub struct FieldView {
	text: Zeroizing<String>,
}

impl FieldView {
	pub fn new(text: Zeroizing<String>) -> Self {
		Self { text }
	}

	pub fn as_str(&self) -> &str {
		self.text.as_str()
	}

	pub fn is_empty(&self) -> bool {
		self.text.is_empty()
	}

	pub fn wrap(&self, width: usize) -> Vec<Zeroizing<String>> {
		wrap_text(self.as_str(), width)
	}
}

pub fn wrap_text(text: &str, width: usize) -> Vec<Zeroizing<String>> {
	let width = width.max(1);
	let mut wrapped = Vec::new();

	for line in text.split('\n') {
		let line = line.strip_suffix('\r').unwrap_or(line);
		let mut current = Zeroizing::new(String::new());

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
			wrapped.push(Zeroizing::new(String::new()));
		}
	}

	wrapped
}

#[cfg(test)]
mod tests {
	use super::wrap_text;

	#[test]
	fn wraps_multiline_notes_to_the_available_width() {
		let wrapped = wrap_text("first line\nsecond line with more text", 12);

		let actual: Vec<&str> = wrapped.iter().map(|line| line.as_str()).collect();

		assert_eq!(actual, ["first line", "second line", "with more", "text"]);
	}

	#[test]
	fn wraps_long_words_instead_of_clipping_them() {
		let wrapped = wrap_text("averylongword", 5);

		let actual: Vec<&str> = wrapped.iter().map(|line| line.as_str()).collect();

		assert_eq!(actual, ["avery", "longw", "ord"]);
	}

	#[test]
	fn preserves_blank_lines_in_notes() {
		let wrapped = wrap_text("first\n\nthird", 20);

		let actual: Vec<&str> = wrapped.iter().map(|line| line.as_str()).collect();

		assert_eq!(actual, ["first", "", "third"]);
	}
}
