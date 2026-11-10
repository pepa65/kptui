use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use ratatui::layout::Size;

use super::{CELL_HEIGHT, HEIGHT, WIDTH};

pub const DEFAULT_SIZE: Size = Size::new(WIDTH, HEIGHT);
pub const MIN_SIZE: Size = Size::new(WIDTH * 3 / 4, HEIGHT * 3 / 4);
pub const MAX_SIZE: Size = Size::new(WIDTH * 4, HEIGHT * 4);

pub struct Viewport {
	pub size: Size,
	pub scale: f64,
	pub offset_x: f64,
	pub offset_y: f64,
}

impl Viewport {
	pub fn new(size: Size) -> Self {
		// Bound the raster allocation even if a window manager ignores size hints.
		let size = Size::new(size.width.clamp(1, MAX_SIZE.width), size.height.clamp(1, MAX_SIZE.height));
		let scale = (f64::from(size.width) / f64::from(WIDTH)).min(f64::from(size.height) / f64::from(HEIGHT));
		let offset_x = (f64::from(size.width) - f64::from(WIDTH) * scale) / 2.0;
		let offset_y = (f64::from(size.height) - f64::from(HEIGHT) * scale) / 2.0;
		Self { size, scale, offset_x, offset_y }
	}

	pub fn grid_position(&self, x: i16, y: i16) -> (f64, f64) {
		let x = (f64::from(x) - self.offset_x) / self.scale;
		let y = (f64::from(y) - self.offset_y) / self.scale;
		(x / f64::from(super::CELL_WIDTH), y / f64::from(CELL_HEIGHT))
	}

	pub fn row_at(&self, x: i16, y: i16) -> Option<usize> {
		let x = (f64::from(x) - self.offset_x) / self.scale;
		let y = (f64::from(y) - self.offset_y) / self.scale;
		if (0.0..f64::from(WIDTH)).contains(&x) && (0.0..f64::from(HEIGHT)).contains(&y) {
			Some((y / f64::from(CELL_HEIGHT)) as usize)
		} else {
			None
		}
	}
}

fn valid_size(size: Size) -> bool {
	(MIN_SIZE.width..=MAX_SIZE.width).contains(&size.width) && (MIN_SIZE.height..=MAX_SIZE.height).contains(&size.height)
}

pub fn load_size(path: &Path) -> Size {
	fn parse(text: &str) -> Option<Size> {
		let (width, height) = text.trim().split_once(' ')?;
		let size = Size::new(width.parse().ok()?, height.parse().ok()?);
		valid_size(size).then_some(size)
	}

	fs::read_to_string(path).ok().and_then(|text| parse(&text)).unwrap_or(DEFAULT_SIZE)
}

pub fn save_size(path: &Path, size: Size) -> io::Result<()> {
	if !valid_size(size) {
		return Ok(());
	}
	// Separate temporary files let two windows close without writing over
	// each other's in-progress size. The last successful rename wins.
	let temp = path.with_extension(format!("{}.tmp", std::process::id()));
	let mut file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
	let result = writeln!(file, "{} {}", size.width, size.height).and_then(|()| fs::rename(&temp, path));
	if result.is_err() {
		let _ = fs::remove_file(temp);
	}
	result
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn scaled_rows_match_draw_coordinates_and_exclude_padding() {
		let wide = Viewport::new(Size::new(WIDTH * 2, HEIGHT));
		assert_eq!(wide.scale, 1.0);
		assert_eq!(wide.row_at(479, 96), None);
		assert_eq!(wide.row_at(480, 96), Some(4));

		let tall = Viewport::new(Size::new(WIDTH, HEIGHT * 2));
		assert_eq!(tall.row_at(0, 311), None);
		assert_eq!(tall.row_at(0, 312 + 95), Some(3));
		assert_eq!(tall.row_at(0, 312 + 96), Some(4));

		let larger = Viewport::new(Size::new(WIDTH * 2, HEIGHT * 2));
		assert_eq!(larger.scale, 2.0);
		assert_eq!(larger.row_at(100, 191), Some(3));
		assert_eq!(larger.row_at(100, 192), Some(4));
	}

	#[test]
	fn last_valid_size_round_trips() -> io::Result<()> {
		let dir = std::env::temp_dir().join(format!("kptui-gui-size-test-{}", std::process::id()));
		fs::create_dir(&dir)?;
		let path = dir.join("gui-size");
		let result = (|| {
			assert_eq!(load_size(&path), DEFAULT_SIZE);
			let size = Size::new(1440, 900);
			save_size(&path, size)?;
			assert_eq!(load_size(&path), size);
			save_size(&path, Size::new(1, 1))?;
			assert_eq!(load_size(&path), size);
			fs::write(&path, "broken")?;
			assert_eq!(load_size(&path), DEFAULT_SIZE);
			Ok(())
		})();
		fs::remove_dir_all(dir)?;
		result
	}
}
