use std::ops::Range;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Wraps `text` to `width` columns and returns byte ranges into `text`.
/// Whitespace is preserved, nothing is copied, and every logical line
/// (including blank ones) yields at least one range.
pub fn wrap_ranges(text: &str, width: usize) -> Vec<Range<usize>> {
	let width = width.max(1);
	let mut out = Vec::new();
	let mut base = 0;

	for raw in text.split('\n') {
		let line = raw.strip_suffix('\r').unwrap_or(raw);
		wrap_line(line, base, width, &mut out);
		base += raw.len() + 1;
	}

	out
}

fn wrap_line(line: &str, base: usize, width: usize, out: &mut Vec<Range<usize>>) {
	let mut start = 0;
	let mut col = 0;
	let mut last_break: Option<usize> = None;

	for (i, ch) in line.char_indices() {
		let w = UnicodeWidthChar::width(ch).unwrap_or(0);

		if col > 0 && col + w > width {
			// Prefer breaking after the last whitespace; otherwise cut the word here.
			let cut = last_break.filter(|&b| b > start).unwrap_or(i);
			out.push(base + start..base + cut);
			start = cut;
			col = 0;
			last_break = None;
			for (j, c) in line[start..i].char_indices() {
				col += UnicodeWidthChar::width(c).unwrap_or(0);
				if c.is_whitespace() {
					last_break = Some(start + j + c.len_utf8());
				}
			}
		}

		col += w;
		if ch.is_whitespace() {
			last_break = Some(i + ch.len_utf8());
		}
	}

	out.push(base + start..base + line.len());
}

/// Row and display column of byte offset `cursor` within the wrapped ranges.
pub fn cursor_position(text: &str, ranges: &[Range<usize>], cursor: usize) -> (usize, usize) {
	let row = ranges.iter().rposition(|r| r.start <= cursor).unwrap_or(0);
	let start = ranges.get(row).map_or(0, |r| r.start);
	(row, UnicodeWidthStr::width(&text[start..cursor.max(start)]))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn lines<'a>(text: &'a str, width: usize) -> Vec<&'a str> {
		wrap_ranges(text, width).into_iter().map(|r| &text[r]).collect()
	}

	#[test]
	fn wraps_multiline_notes_to_the_available_width() {
		assert_eq!(lines("first line\nsecond line with more text", 12), ["first line", "second line ", "with more ", "text"]);
	}

	#[test]
	fn wraps_long_words_instead_of_clipping_them() {
		assert_eq!(lines("averylongword", 5), ["avery", "longw", "ord"]);
	}

	#[test]
	fn preserves_blank_lines_in_notes() {
		assert_eq!(lines("first\n\nthird", 20), ["first", "", "third"]);
	}

	#[test]
	fn preserves_spaces_and_indentation() {
		assert_eq!(lines("  a  b", 20), ["  a  b"]);
	}

	#[test]
	fn cursor_follows_trailing_spaces_and_newlines() {
		let text = "ab  \ncd";
		let ranges = wrap_ranges(text, 20);
		assert_eq!(cursor_position(text, &ranges, 4), (0, 4)); // after the two spaces
		assert_eq!(cursor_position(text, &ranges, 5), (1, 0)); // start of "cd"
		assert_eq!(cursor_position(text, &ranges, 7), (1, 2)); // end of text
	}
}
