use anyhow::{Context, ensure};
use ratatui::layout::Size;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::properties::WmSizeHints;
use x11rb::protocol::{Event, xkb, xproto::*};
use x11rb::wrapper::ConnectionExt as _;

use super::{
	Input, SelectionChange, Session,
	keyboard::Keyboard,
	primary::Primary,
	viewport::{MAX_SIZE, MIN_SIZE},
};

pub(super) fn run() -> anyhow::Result<()> {
	let (conn, screen_index) = x11rb::connect(None).context("could not connect to the X11 display")?;
	let setup = conn.setup();
	let screen = &setup.roots[screen_index];
	let byte_order = if cfg!(target_endian = "little") { ImageOrder::LSB_FIRST } else { ImageOrder::MSB_FIRST };
	let rgb = screen.allowed_depths.iter().flat_map(|depth| &depth.visuals).any(|visual| {
		visual.visual_id == screen.root_visual
			&& visual.class == VisualClass::TRUE_COLOR
			&& (visual.red_mask, visual.green_mask, visual.blue_mask) == (0xff0000, 0x00ff00, 0x0000ff)
	});
	ensure!(
		screen.root_depth == 24
			&& setup.image_byte_order == byte_order
			&& rgb && setup.pixmap_formats.iter().any(|f| f.depth == 24 && f.bits_per_pixel == 32 && f.scanline_pad == 32),
		"GUI requires a native-byte-order 24-bit TrueColor display with 32-bit pixels"
	);

	let mut session = Session::new()?;
	let initial_size = session.size;
	let max_image_bytes = conn.maximum_request_bytes().min(256 * 1024).saturating_sub(32);
	let window = conn.generate_id()?;
	let gc = conn.generate_id()?;
	let events = EventMask::EXPOSURE
		| EventMask::KEY_PRESS
		| EventMask::BUTTON_PRESS
		| EventMask::BUTTON_RELEASE
		| EventMask::BUTTON1_MOTION
		| EventMask::STRUCTURE_NOTIFY
		| EventMask::FOCUS_CHANGE;
	conn.create_window(
		screen.root_depth,
		window,
		screen.root,
		0,
		0,
		initial_size.width,
		initial_size.height,
		0,
		WindowClass::INPUT_OUTPUT,
		screen.root_visual,
		&CreateWindowAux::new().background_pixel(0x101923).event_mask(events),
	)?
	.check()?;
	conn.create_gc(gc, window, &CreateGCAux::new())?.check()?;
	conn.change_property8(PropMode::REPLACE, window, AtomEnum::WM_NAME, AtomEnum::STRING, crate::BIN_NAME.as_bytes())?
		.check()?;
	let protocols = conn.intern_atom(false, b"WM_PROTOCOLS")?.reply()?.atom;
	let close = conn.intern_atom(false, b"WM_DELETE_WINDOW")?.reply()?.atom;
	conn.change_property32(PropMode::REPLACE, window, protocols, AtomEnum::ATOM, &[close])?.check()?;
	let mut hints = WmSizeHints::new();
	hints.min_size = Some((MIN_SIZE.width.into(), MIN_SIZE.height.into()));
	hints.max_size = Some((MAX_SIZE.width.into(), MAX_SIZE.height.into()));
	hints.set_normal_hints(&conn, window)?.check()?;

	ensure!(xkb::use_extension(&conn, 1, 0)?.reply()?.supported, "the X11 server does not support XKB");
	let map = xkb::MapPart::KEY_TYPES | xkb::MapPart::KEY_SYMS | xkb::MapPart::MODIFIER_MAP | xkb::MapPart::VIRTUAL_MODS | xkb::MapPart::VIRTUAL_MOD_MAP;
	xkb::select_events(
		&conn,
		xkb::ID::USE_CORE_KBD.into(),
		xkb::EventType::default(),
		xkb::EventType::NEW_KEYBOARD_NOTIFY | xkb::EventType::MAP_NOTIFY,
		map,
		map,
		&xkb::SelectEventsAux::default(),
	)?
	.check()?;
	let mut primary = Primary::new(&conn, screen.root)?;
	let mut keyboard = Keyboard::open().map_err(anyhow::Error::msg)?;
	conn.map_window(window)?.check()?;
	conn.flush()?;
	let mut fds = [PollFd::new(conn.stream(), PollFlags::IN)];
	let timeout = Timespec { tv_sec: 0, tv_nsec: 200_000_000 };
	while !session.should_close() {
		let event = if session.pending_save() { None } else { conn.poll_for_event()? };
		if event.is_none() {
			if session.redraw {
				let (pixels, stride, width) = session.draw()?;
				let rows_per_request = max_image_bytes / stride;
				ensure!(rows_per_request > 0, "X11 request limit is too small for the window width");
				for (chunk, rows) in pixels.chunks(stride * rows_per_request).enumerate() {
					conn.put_image(
						ImageFormat::Z_PIXMAP,
						window,
						gc,
						width,
						(rows.len() / stride) as u16,
						0,
						(chunk * rows_per_request) as i16,
						0,
						screen.root_depth,
						rows,
					)?;
				}
				conn.flush()?;
			}
			if session.save_if_pending() {
				continue;
			}
			let timed_out = match poll(&mut fds, Some(&timeout)) {
				Ok(n) => n == 0,
				Err(rustix::io::Errno::INTR) => false,
				Err(err) => return Err(err.into()),
			};
			if timed_out {
				session.tick();
			}
			continue;
		}

		let mut time = x11rb::CURRENT_TIME;
		let input = match event.unwrap() {
			Event::Expose(event) if event.count == 0 => {
				session.redraw = true;
				None
			}
			Event::ConfigureNotify(event) if event.window == window => {
				session.resize(Size::new(event.width, event.height));
				None
			}
			Event::FocusOut(_) => Some(Input::FocusLost),
			Event::KeyPress(event) => keyboard.key_event(event.detail, event.state).map(Input::Key),
			Event::ButtonPress(event) => Some(Input::Press { button: event.detail, x: event.event_x, y: event.event_y, time: event.time }),
			Event::MotionNotify(event) if event.state.contains(KeyButMask::BUTTON1) => Some(Input::Motion { x: event.event_x, y: event.event_y }),
			Event::ButtonRelease(event) if event.detail == 1 => {
				time = event.time;
				Some(Input::Release { x: event.event_x, y: event.event_y })
			}
			Event::SelectionRequest(event) => {
				primary.respond(&conn, event)?;
				None
			}
			Event::SelectionClear(event) => {
				if primary.lost(&conn, event)? {
					session.selection_lost();
				}
				None
			}
			Event::XkbMapNotify(_) | Event::XkbNewKeyboardNotify(_) => {
				keyboard = Keyboard::open().map_err(anyhow::Error::msg)?;
				None
			}
			Event::ClientMessage(event) if event.type_ == protocols && event.format == 32 && event.data.as_data32()[0] == close => break,
			Event::DestroyNotify(event) if event.window == window => break,
			Event::Error(error) => anyhow::bail!("X11 request failed: {error:?}"),
			_ => None,
		};
		if let Some(input) = input {
			match session.input(input) {
				SelectionChange::Keep => {}
				SelectionChange::Clear => primary.clear(&conn)?,
				SelectionChange::Publish => {
					let text = session.selected_text();
					if !primary.publish(&conn, text, time)? && !text.is_empty() {
						eprintln!("{}: could not publish this selection to X11 PRIMARY", crate::BIN_NAME);
					}
				}
			}
		}
	}
	session.persist_size();
	Ok(())
}
