mod keyboard;
mod paint;
mod primary;
mod viewport;
mod wayland;
mod x11;

use std::convert::Infallible;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
	Terminal,
	backend::{Backend, WindowSize},
	buffer::{Buffer, Cell, CellWidth},
	layout::{Position, Rect, Size},
	style::Color,
};
use zeroize::{Zeroize, Zeroizing};

use crate::app::{self, Screen};
use crate::input::{SECRET_CAP, edit::process_pending_save};
use crate::ui::edit::{EditView, field_at_position, readonly_view};
use crate::ui::index::{SEARCH_HEIGHT, header_height};
use paint::Canvas;
#[cfg(test)]
use viewport::MIN_SIZE;
use viewport::{Viewport, load_size, save_size};

const GRID: Size = Size::new(96, 26);
const CELL_WIDTH: u16 = 10;
const CELL_HEIGHT: u16 = 24;
const WIDTH: u16 = GRID.width * CELL_WIDTH;
const HEIGHT: u16 = GRID.height * CELL_HEIGHT;
const DOUBLE_CLICK_MS: u32 = 400;

const GRID_AREA: Rect = Rect::new(0, 0, GRID.width, GRID.height);

pub fn run() -> anyhow::Result<()> {
	match std::env::var("KPGUI_BACKEND").as_deref() {
		Ok("x11") => x11::run(),
		Ok("wayland") => wayland::run(),
		Ok(other) => anyhow::bail!("unknown KPGUI_BACKEND {other:?}; use x11 or wayland"),
		Err(std::env::VarError::NotPresent)
			if ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"].iter().any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty())) =>
		{
			wayland::run()
		}
		Err(std::env::VarError::NotPresent) => x11::run(),
		Err(err) => anyhow::bail!("{err}"),
	}
}

enum Input {
	FocusLost,
	Key(KeyEvent),
	Press { button: u8, x: i16, y: i16, time: u32 },
	Motion { x: i16, y: i16 },
	Release { x: i16, y: i16 },
}

enum SelectionChange {
	Keep,
	Clear,
	Publish,
}

struct Session {
	app: app::App,
	terminal: Terminal<FrameBackend>,
	canvas: Canvas,
	viewport: Viewport,
	size_path: std::path::PathBuf,
	initial_size: Size,
	size: Size,
	redraw: bool,
	last_click: Option<(ClickTarget, u32)>,
	edit_view: Option<EditView>,
	dragging: bool,
	readonly_press: Option<ReadonlyPress>,
	readonly_selection: Option<ReadonlySelection>,
}

impl Session {
	fn new() -> anyhow::Result<Self> {
		let size_path = crate::util::expand_tilde("~/.config/kptui/gui-size");
		let size = load_size(&size_path);
		let viewport = Viewport::new(size);
		Ok(Self {
			app: app::start(false, Zeroizing::new(String::with_capacity(SECRET_CAP))),
			terminal: Terminal::new(FrameBackend::default())?,
			canvas: Canvas::new(&viewport).map_err(anyhow::Error::msg)?,
			viewport,
			size_path,
			initial_size: size,
			size,
			redraw: true,
			last_click: None,
			edit_view: None,
			dragging: false,
			readonly_press: None,
			readonly_selection: None,
		})
	}

	fn should_close(&mut self) -> bool {
		app::maybe_auto_exit(&mut self.app);
		self.app.should_quit
	}

	fn pending_save(&self) -> bool {
		self.app.pending_save
	}

	fn save_if_pending(&mut self) -> bool {
		if !self.pending_save() {
			return false;
		}
		process_pending_save(&mut self.app);
		self.redraw = true;
		true
	}

	fn resize(&mut self, size: Size) {
		if size != self.size {
			self.input(Input::FocusLost);
			self.size = size;
			self.viewport = Viewport::new(size);
			self.redraw = true;
		}
	}

	fn persist_size(&self) {
		if self.size != self.initial_size
			&& let Err(error) = save_size(&self.size_path, self.size)
		{
			eprintln!("{}: could not save GUI window size: {error}", crate::BIN_NAME);
		}
	}

	fn tick(&mut self) {
		self.redraw |= self.app.screen == Screen::Edit && self.app.totp_cache.is_some();
	}

	fn selection_lost(&mut self) {
		self.readonly_selection = None;
		self.redraw = true;
	}

	fn selected_text(&self) -> &str {
		if self.app.editing_field {
			self.app.field_editor.as_ref().and_then(|editor| editor.selection().and_then(|range| editor.text().get(range)))
		} else {
			self.readonly_selection.as_ref().and_then(|selection| {
				let range = selection.range()?;
				let (_, text) = readonly_view(GRID_AREA, &self.app, selection.field, None)?;
				text.get(range)
			})
		}
		.unwrap_or_default()
	}

	fn draw(&mut self) -> anyhow::Result<(&[u8], usize, u16)> {
		if self.canvas.size() != self.viewport.size {
			self.canvas = Canvas::new(&self.viewport).map_err(anyhow::Error::msg)?;
		}
		self.canvas.clear(rgb_color(self.app.theme.background, 0x101923));
		let frame = self.terminal.draw(|frame| self.edit_view = app::draw(frame, &mut self.app))?;
		paint_frame(&self.canvas, frame.buffer, self.app.theme.background, self.app.theme.text);
		let readonly_highlight = self.readonly_selection.as_ref().and_then(|selection| {
			let range = selection.range()?;
			readonly_view(GRID_AREA, &self.app, selection.field, Some(&range)).map(|(view, _)| view)
		});
		if let Some(view) = self.edit_view.as_ref().or(readonly_highlight.as_ref()) {
			let color = rgb_color(self.app.theme.selection_bg, 0x89b4fa);
			for rect in &view.highlights {
				self.canvas.selection_overlay(
					i32::from(rect.x * CELL_WIDTH),
					i32::from(rect.y * CELL_HEIGHT),
					i32::from(rect.width * CELL_WIDTH),
					i32::from(rect.height * CELL_HEIGHT),
					color,
				);
			}
		}
		if let Some(cursor) = self.terminal.backend().visible_cursor() {
			self.canvas.rect(
				(cursor.x * CELL_WIDTH).into(),
				(cursor.y * CELL_HEIGHT + CELL_HEIGHT - 3).into(),
				CELL_WIDTH.into(),
				2,
				rgb_color(self.app.theme.text, 0xffffff),
			);
		}
		// Discard the completed frame instead of retaining it for diffing.
		self.terminal.clear()?;
		self.redraw = false;
		let (pixels, stride) = self.canvas.pixels().map_err(anyhow::Error::msg)?;
		Ok((pixels, stride, self.viewport.size.width))
	}

	fn input(&mut self, input: Input) -> SelectionChange {
		match input {
			Input::FocusLost => {
				self.last_click = None;
				self.dragging = false;
				self.readonly_press = None;
			}
			Input::Key(key) => {
				self.last_click = None;
				self.dragging = false;
				self.readonly_press = None;
				self.edit_view = None;
				self.readonly_selection = None;
				let was_editing = self.app.editing_field;
				app::handle_key(&mut self.app, key);
				self.redraw = true;
				if self.app.screen != Screen::Edit || was_editing != self.app.editing_field {
					return SelectionChange::Clear;
				}
			}
			Input::Press { button, x, y, time } if self.app.screen == Screen::Index && self.app.confirm_index_delete.is_none() => match button {
				1 => {
					let first_row = usize::from(SEARCH_HEIGHT + header_height(self.app.slim_mode));
					let row = self.viewport.row_at(x, y).and_then(|row| row.checked_sub(first_row));
					let selected = row.filter(|&row| row < self.app.index_page_size).map(|row| self.app.index_state.offset() + row);
					let click = selected.and_then(|row| self.app.filtered.get(row).map(|&entry| (row, entry)));
					let open = double_click(&mut self.last_click, click.map(|(_, entry)| ClickTarget::Entry(entry)), time);
					if let Some((row, _)) = click {
						self.app.last_activity = Instant::now();
						self.app.index_state.select(Some(row));
						self.app.status = None;
						if open {
							app::handle_key(&mut self.app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
						}
						self.redraw = true;
					}
				}
				4 | 5 => {
					self.last_click = None;
					let code = if button == 4 { KeyCode::Up } else { KeyCode::Down };
					app::handle_key(&mut self.app, KeyEvent::new(code, KeyModifiers::NONE));
					self.redraw = true;
				}
				_ => self.last_click = None,
			},
			Input::Press { button: 1, x, y, time } if self.app.screen == Screen::Edit && !self.app.editing_field && !self.app.confirm_exit => {
				self.readonly_press = None;
				self.redraw |= self.readonly_selection.take().is_some();
				let (column, row) = self.viewport.grid_position(x, y);
				let target = self
					.viewport
					.row_at(x, y)
					.and_then(|row| field_at_position(GRID_AREA, self.app.slim_mode, column, row as u16))
					.and_then(|(field, on_label)| {
						if on_label {
							self.app.edit_state.select(Some(field));
							self.app.status = None;
							self.redraw = true;
							return Some(ReadonlyTarget::Label(field));
						}
						let (view, text) = readonly_view(GRID_AREA, &self.app, field, None)?;
						let line = view.line_at(row)?;
						let offset = view.offset_at(text, column, row, false)?;
						self.readonly_selection = Some(ReadonlySelection { field, anchor: offset, end: offset });
						Some(ReadonlyTarget::Line { field, start: line.start, end: line.end })
					});
				if let Some(target) = target {
					self.app.last_activity = Instant::now();
					self.readonly_press = Some(ReadonlyPress { target, time, origin: (x, y), moved: false });
				} else {
					self.last_click = None;
				}
			}
			Input::Press { button: 1, x, y, .. } if self.edit_view.is_some() && self.app.editing_field => {
				self.last_click = None;
				self.dragging = false;
				if let (Some(view), Some(editor)) = (&self.edit_view, &mut self.app.field_editor) {
					let (column, row) = self.viewport.grid_position(x, y);
					if let Some(offset) = view.offset_at(editor.text(), column, row, false) {
						editor.select_to(offset, false);
						self.dragging = true;
						self.app.last_activity = Instant::now();
						self.redraw = true;
					}
				}
			}
			Input::Motion { x, y } if self.dragging => {
				if let (Some(view), Some(editor)) = (&self.edit_view, &mut self.app.field_editor) {
					let (column, row) = self.viewport.grid_position(x, y);
					if let Some(offset) = view.offset_at(editor.text(), column, row, true) {
						editor.select_to(offset, true);
						self.app.last_activity = Instant::now();
						self.redraw = true;
					}
				}
			}
			Input::Motion { x, y } if self.readonly_press.is_some() => {
				if let Some(press) = &mut self.readonly_press
					&& press.track_motion(x, y)
				{
					self.last_click = None;
					if let Some(selection) = &mut self.readonly_selection {
						self.redraw |= selection.extend(&self.app, &self.viewport, GRID_AREA, x, y);
						self.app.last_activity = Instant::now();
					}
				}
			}
			Input::Release { x, y } => {
				if std::mem::take(&mut self.dragging) {
					return SelectionChange::Publish;
				}
				if let Some(mut press) = self.readonly_press.take() {
					let clicked = !press.track_motion(x, y);
					if !clicked && let Some(selection) = &mut self.readonly_selection {
						self.redraw |= selection.extend(&self.app, &self.viewport, GRID_AREA, x, y);
					}
					let target = clicked.then_some(ClickTarget::Readonly(press.target));
					if double_click(&mut self.last_click, target, press.time) {
						match press.target {
							ReadonlyTarget::Label(_) => {
								self.readonly_selection = None;
								app::handle_key(&mut self.app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
							}
							ReadonlyTarget::Line { field, start, end } => {
								self.readonly_selection = Some(ReadonlySelection { field, anchor: start, end });
							}
						}
						self.redraw = true;
					}
					return SelectionChange::Publish;
				}
			}
			Input::Press { .. } => self.last_click = None,
			_ => {}
		}
		SelectionChange::Keep
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadonlyTarget {
	Label(usize),
	Line { field: usize, start: usize, end: usize },
}

struct ReadonlyPress {
	target: ReadonlyTarget,
	time: u32,
	origin: (i16, i16),
	moved: bool,
}

impl ReadonlyPress {
	fn track_motion(&mut self, x: i16, y: i16) -> bool {
		self.moved |= self.origin.0.abs_diff(x) >= 4 || self.origin.1.abs_diff(y) >= 4;
		self.moved
	}
}

struct ReadonlySelection {
	field: usize,
	anchor: usize,
	end: usize,
}

impl ReadonlySelection {
	fn extend(&mut self, app: &app::App, viewport: &Viewport, area: Rect, x: i16, y: i16) -> bool {
		let Some((view, text)) = readonly_view(area, app, self.field, None) else { return false };
		let (column, row) = viewport.grid_position(x, y);
		let Some(offset) = view.offset_at(text, column, row, true) else { return false };
		let changed = self.end != offset;
		self.end = offset;
		changed
	}

	fn range(&self) -> Option<std::ops::Range<usize>> {
		(self.anchor != self.end).then(|| self.anchor.min(self.end)..self.anchor.max(self.end))
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClickTarget {
	Entry(usize),
	Readonly(ReadonlyTarget),
}

fn double_click(previous: &mut Option<(ClickTarget, u32)>, target: Option<ClickTarget>, time: u32) -> bool {
	let first = previous.take();
	let Some(target) = target else { return false };
	// Display-server timestamps are milliseconds and wrap at 2^32.
	if first.is_some_and(|(clicked, then)| clicked == target && time.wrapping_sub(then) <= DOUBLE_CLICK_MS) {
		return true;
	}
	*previous = Some((target, time));
	false
}

fn rgb_color(color: Color, fallback: u32) -> u32 {
	match color {
		Color::Rgb(r, g, b) => u32::from_be_bytes([0, r, g, b]),
		_ => fallback, // kptui themes supply RGB colors.
	}
}

fn paint_frame(canvas: &Canvas, buffer: &Buffer, background: Color, text: Color) {
	let background = rgb_color(background, 0x101923);
	let text = rgb_color(text, 0xf1f5f9);
	for (y, row) in buffer.content.chunks_exact(usize::from(GRID.width)).enumerate() {
		let y = y as i32 * i32::from(CELL_HEIGHT);
		// Reuse a preallocated buffer so appending plaintext never reallocates.
		let mut run = Zeroizing::new(String::with_capacity(row.iter().map(|cell| cell.symbol().len()).sum()));
		let mut x = 0;
		while x < row.len() {
			let style = (row[x].fg, row[x].bg);
			let mut end = x;
			while let Some(cell) = row.get(end).filter(|cell| (cell.fg, cell.bg) == style) {
				run.push_str(cell.symbol());
				// Ratatui resets continuation cells, including their style.
				// Keep them in the same run as the glyph that occupies them.
				end += usize::from(cell.cell_width().max(1));
			}
			end = end.min(row.len());
			let px = x as i32 * i32::from(CELL_WIDTH);
			let width = (end - x) as i32 * i32::from(CELL_WIDTH);
			canvas.rect(px, y, width, CELL_HEIGHT.into(), rgb_color(style.1, background));

			let visible = run.trim_end_matches(' ');
			if !visible.is_empty() {
				canvas.text(px, y + 1, width, visible, rgb_color(style.0, text));
			}
			run.zeroize();
			x = end;
		}
	}
}

// Ratatui supplies the layout and cursor for the rasterizer. This backend
// intentionally holds no copy of the terminal cells.
#[derive(Default)]
struct FrameBackend {
	cursor: Position,
	cursor_shown: bool,
}

impl FrameBackend {
	fn visible_cursor(&self) -> Option<Position> {
		self.cursor_shown.then_some(self.cursor).filter(|pos| pos.x < GRID.width && pos.y < GRID.height)
	}
}

impl Backend for FrameBackend {
	type Error = Infallible;

	fn draw<'a, I: Iterator<Item = (u16, u16, &'a Cell)>>(&mut self, _content: I) -> Result<(), Self::Error> {
		Ok(())
	}
	fn hide_cursor(&mut self) -> Result<(), Self::Error> {
		self.cursor_shown = false;
		Ok(())
	}
	fn show_cursor(&mut self) -> Result<(), Self::Error> {
		self.cursor_shown = true;
		Ok(())
	}
	fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
		Ok(self.cursor)
	}
	fn set_cursor_position<P: Into<Position>>(&mut self, pos: P) -> Result<(), Self::Error> {
		self.cursor = pos.into();
		Ok(())
	}
	fn clear(&mut self) -> Result<(), Self::Error> {
		Ok(())
	}
	fn clear_region(&mut self, _: ratatui::backend::ClearType) -> Result<(), Self::Error> {
		Ok(())
	}
	fn size(&self) -> Result<Size, Self::Error> {
		Ok(GRID)
	}
	fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
		Ok(WindowSize { columns_rows: GRID, pixels: Size::new(WIDTH, HEIGHT) })
	}
	fn flush(&mut self) -> Result<(), Self::Error> {
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn readonly_drag_does_not_count_as_a_click_even_if_it_returns_to_its_start() {
		let mut press = ReadonlyPress { target: ReadonlyTarget::Label(2), time: 100, origin: (20, 30), moved: false };
		assert!(!press.track_motion(23, 30));
		assert!(press.track_motion(24, 30));
		assert!(press.track_motion(20, 30));
	}

	#[test]
	fn readonly_selection_handles_reverse_and_empty_ranges() {
		let mut selection = ReadonlySelection { field: 5, anchor: 12, end: 3 };
		assert_eq!(selection.range(), Some(3..12));
		selection.end = 12;
		assert_eq!(selection.range(), None);
	}

	#[test]
	fn double_click_requires_two_recent_clicks_on_the_same_entry() {
		let mut previous = None;
		assert!(!double_click(&mut previous, Some(ClickTarget::Entry(1)), 100));
		assert!(!double_click(&mut previous, Some(ClickTarget::Entry(2)), 150));
		assert!(!double_click(&mut previous, Some(ClickTarget::Entry(2)), 551));
		assert!(double_click(&mut previous, Some(ClickTarget::Entry(2)), 700));
		assert!(!double_click(&mut previous, Some(ClickTarget::Entry(2)), 750));
		assert!(!double_click(&mut previous, None, 800));
		assert!(!double_click(&mut previous, Some(ClickTarget::Entry(2)), 850));

		previous = Some((ClickTarget::Entry(3), u32::MAX - 99));
		assert!(double_click(&mut previous, Some(ClickTarget::Entry(3)), 100));
		assert!(!double_click(&mut previous, Some(ClickTarget::Entry(1)), 200));
		assert!(!double_click(&mut previous, Some(ClickTarget::Readonly(ReadonlyTarget::Label(1))), 250));
		assert!(double_click(&mut previous, Some(ClickTarget::Readonly(ReadonlyTarget::Label(1))), 300));
	}

	#[test]
	fn label_and_value_clicks_do_not_form_a_pair() {
		let mut previous = None;
		let label = ClickTarget::Readonly(ReadonlyTarget::Label(2));
		let line = ClickTarget::Readonly(ReadonlyTarget::Line { field: 2, start: 0, end: 10 });
		assert!(!double_click(&mut previous, Some(label), 100));
		assert!(!double_click(&mut previous, Some(line), 150));
		assert!(double_click(&mut previous, Some(line), 200));
	}

	#[test]
	fn entry_hit_test_distinguishes_labels_and_values() {
		let area = Rect::new(0, 0, GRID.width, GRID.height);
		let hit = |column, row| field_at_position(area, false, column, row);
		assert_eq!(hit(0.0, 0), None);
		for field in 0..5 {
			assert_eq!(hit(0.0, field as u16 + 1), Some((field, true)));
			assert_eq!(hit(20.0, field as u16 + 1), Some((field, false)));
		}
		assert_eq!(hit(0.0, 6), Some((5, true)));
		assert_eq!(hit(20.0, 6), Some((5, false)));
		assert_eq!(hit(0.0, 7), Some((5, false)));
		assert_eq!(hit(0.0, GRID.height - 2), Some((5, false)));
		assert_eq!(hit(0.0, GRID.height - 1), None);
	}

	#[test]
	fn thai_continuation_cells_do_not_split_paint_runs() -> anyhow::Result<()> {
		use ratatui::{
			style::Style,
			text::Span,
			widgets::{List, Paragraph, Widget},
		};

		let viewport = Viewport::new(viewport::DEFAULT_SIZE);
		let style = Style::new().fg(Color::White);
		let mut view = Buffer::empty(GRID_AREA);
		List::new([Span::styled("ลำ", style)]).render(Rect::new(0, 0, GRID.width, 1), &mut view);
		Paragraph::new(Span::styled("ลำดับ น้ำขึ้น", style)).render(Rect::new(0, 1, GRID.width, 1), &mut view);

		// The editor applies a base style to the whole field, including
		// the blank continuation cells. Match their foreground here to
		// isolate the effect of those cells on rasterization.
		let mut uniform = view.clone();
		for (x, y) in [(1, 0), (1, 1), (6, 1)] {
			assert_eq!(view[(x, y)].symbol(), " ");
			assert_ne!(view[(x, y)].fg, Color::White);
			uniform[(x, y)].set_style(style);
		}

		let mut actual = Canvas::new(&viewport).map_err(anyhow::Error::msg)?;
		let mut expected = Canvas::new(&viewport).map_err(anyhow::Error::msg)?;
		for (canvas, buffer) in [(&actual, &view), (&expected, &uniform)] {
			paint_frame(canvas, buffer, Color::Black, Color::White);
		}
		let (actual, _) = actual.pixels().map_err(anyhow::Error::msg)?;
		let (expected, _) = expected.pixels().map_err(anyhow::Error::msg)?;
		assert!(actual == expected, "Thai glyphs should be painted together in view mode");
		Ok(())
	}

	#[test]
	fn rasterizes_thai_text_headlessly() -> anyhow::Result<()> {
		let mut terminal = Terminal::new(FrameBackend::default())?;
		let frame = terminal.draw(|frame| {
			let area = frame.area();
			frame.buffer_mut().set_string(0, 0, "น้ำขึ้น", ratatui::style::Style::default());
			assert_eq!(area.width, GRID.width);
		})?;
		let mut previous_ink = 0;
		for size in [MIN_SIZE, viewport::DEFAULT_SIZE, Size::new(WIDTH * 2, HEIGHT * 2)] {
			let viewport = Viewport::new(size);
			let mut canvas = Canvas::new(&viewport).map_err(anyhow::Error::msg)?;
			paint_frame(&canvas, frame.buffer, Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255));
			let (pixels, stride) = canvas.pixels().map_err(anyhow::Error::msg)?;
			assert_eq!(pixels.len(), stride * usize::from(size.height));
			let row_end = (f64::from(CELL_HEIGHT) * viewport.scale) as usize * stride;
			let ink = pixels[..row_end]
				.chunks_exact(4)
				.filter(|pixel| u32::from_ne_bytes((*pixel).try_into().unwrap()) & 0x00ff_ffff != 0)
				.count();
			assert!(ink > previous_ink, "Thai glyphs should gain pixels as the window grows");
			previous_ink = ink;
		}
		Ok(())
	}
}
