use std::fs::File;
use std::io::{self, Write};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, ensure};
use crossterm::event::KeyModifiers;
use ratatui::layout::Size;
use sctk::{
	compositor::{CompositorHandler, CompositorState, FrameCallbackData},
	data_device_manager::WritePipe,
	output::{OutputHandler, OutputState},
	primary_selection::{
		PrimarySelectionManagerState,
		device::{PrimarySelectionDevice, PrimarySelectionDeviceHandler},
		selection::{PrimarySelectionSource, PrimarySelectionSourceHandler},
	},
	reexports::{
		calloop::EventLoop,
		calloop_wayland_source::WaylandSource,
		csd_frame::{DecorationsFrame, FrameAction, FrameClick, ResizeEdge},
		protocols::{
			wp::primary_selection::zv1::client::{zwp_primary_selection_device_v1, zwp_primary_selection_source_v1},
			xdg::shell::client::xdg_toplevel,
		},
	},
	registry::{ProvidesRegistryState, RegistryState},
	seat::{
		Capability, SeatHandler, SeatState,
		pointer::{CursorIcon, PointerData, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer},
	},
	shell::{
		WaylandSurface,
		xdg::{
			XdgShell, XdgSurface,
			fallback_frame::FallbackFrame,
			window::{DecorationMode, Window, WindowConfigure, WindowDecorations, WindowHandler},
		},
	},
	shm::{
		Shm, ShmHandler,
		slot::{Buffer, SlotPool},
	},
	subcompositor::SubcompositorState,
};
use smithay_client_toolkit as sctk;
use wayland_client::{
	Connection, Dispatch, Proxy, QueueHandle, WEnum,
	globals::registry_queue_init,
	protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};
use xkbcommon::xkb;
use zeroize::{Zeroize, Zeroizing};

use super::{
	Input, SelectionChange, Session,
	keyboard::translate,
	viewport::{MAX_SIZE, MIN_SIZE},
};

const TEXT_TYPES: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];
const TICK: Duration = Duration::from_millis(200);
const LEFT_BUTTON: u32 = 0x110;
const RIGHT_BUTTON: u32 = 0x111;

pub(super) fn run() -> anyhow::Result<()> {
	let conn = Connection::connect_to_env().context("could not connect to the Wayland compositor")?;
	let (globals, queue) = registry_queue_init(&conn)?;
	let qh = queue.handle();
	let mut events: EventLoop<Client> = EventLoop::try_new()?;
	WaylandSource::new(conn.clone(), queue).insert(events.handle())?;

	let compositor = CompositorState::bind(&globals, &qh)?;
	let subcompositor = Arc::new(SubcompositorState::bind(compositor.wl_compositor().clone(), &globals, &qh)?);
	let shell = XdgShell::bind(&globals, &qh)?;
	let shm = Shm::bind(&globals, &qh)?;
	let window = shell.create_window(compositor.create_surface(&qh), WindowDecorations::RequestServer, &qh);
	window.set_title(crate::BIN_NAME);
	window.set_app_id("kpgui");
	window.set_min_size(Some((MIN_SIZE.width.into(), MIN_SIZE.height.into())));
	window.set_max_size(Some((MAX_SIZE.width.into(), MAX_SIZE.height.into())));
	window.commit();

	let primary_manager = PrimarySelectionManagerState::bind(&globals, &qh).ok();
	if primary_manager.is_none() {
		eprintln!("{}: this Wayland compositor does not offer primary selection", crate::BIN_NAME);
	}
	let session = Session::new()?;
	let pool = SlotPool::new(usize::from(session.size.width) * usize::from(session.size.height) * 4, &shm)?;
	let mut client = Client {
		session,
		window,
		compositor,
		subcompositor,
		shm,
		pool,
		buffers: Vec::new(),
		registry: RegistryState::new(&globals),
		seats: SeatState::new(&globals, &qh),
		outputs: OutputState::new(&globals, &qh),
		seat: None,
		keyboard: None,
		pointer: None,
		frame: None,
		keymap: None,
		modifiers: KeyModifiers::NONE,
		repeat: None,
		repeat_rate: 0,
		repeat_delay: 0,
		primary_manager,
		primary_device: None,
		selection: None,
		configured: false,
		frame_pending: false,
		closed: false,
	};

	let mut next_tick = Instant::now() + TICK;
	while !client.closed && !client.session.should_close() {
		let now = Instant::now();
		if now >= next_tick {
			client.session.tick();
			next_tick = now + TICK;
		}
		client.repeat_key();
		client.pump_selection();
		client.wipe_released_buffers();
		if client.configured && !client.frame_pending && client.session.redraw {
			client.draw(&qh)?;
		}
		client.session.save_if_pending();
		let mut timeout = next_tick.saturating_duration_since(Instant::now());
		if client.selection.as_ref().is_some_and(|s| s.transfer.is_some()) {
			timeout = timeout.min(Duration::from_millis(20));
		}
		if let Some((_, due)) = client.repeat {
			timeout = timeout.min(due.saturating_duration_since(Instant::now()));
		}
		events.dispatch(timeout, &mut client)?;
	}
	client.closed = true;
	client.selection = None;
	client.session.persist_size();
	// Give the compositor a chance to release its last frame before unmapping
	// the pool. While it owns a buffer, we must leave those pixels untouched.
	client.window.wl_surface().attach(None, 0, 0);
	client.window.commit();
	let _ = conn.flush();
	let _ = events.dispatch(Duration::from_millis(50), &mut client);
	client.wipe_released_buffers();
	Ok(())
}

struct SharedFrame {
	size: Size,
	buffer: Buffer,
	dirty: bool,
}

struct Selection {
	source: PrimarySelectionSource,
	bytes: Zeroizing<Vec<u8>>,
	transfer: Option<(File, usize, Instant)>,
}

struct Client {
	session: Session,
	window: Window,
	compositor: CompositorState,
	subcompositor: Arc<SubcompositorState>,
	shm: Shm,
	pool: SlotPool,
	buffers: Vec<SharedFrame>,
	registry: RegistryState,
	seats: SeatState,
	outputs: OutputState,
	seat: Option<wl_seat::WlSeat>,
	keyboard: Option<wl_keyboard::WlKeyboard>,
	pointer: Option<ThemedPointer>,
	frame: Option<FallbackFrame<Self>>,
	keymap: Option<xkb::State>,
	modifiers: KeyModifiers,
	repeat: Option<(xkb::Keycode, Instant)>,
	repeat_rate: i32,
	repeat_delay: i32,
	primary_manager: Option<PrimarySelectionManagerState>,
	primary_device: Option<PrimarySelectionDevice>,
	selection: Option<Selection>,
	configured: bool,
	frame_pending: bool,
	closed: bool,
}

impl Client {
	fn wipe_released_buffers(&mut self) {
		for frame in &mut self.buffers {
			if frame.dirty
				&& let Some(bytes) = self.pool.canvas(&frame.buffer)
			{
				bytes.zeroize();
				frame.dirty = false;
			}
		}
	}

	fn draw(&mut self, qh: &QueueHandle<Self>) -> anyhow::Result<()> {
		let size = self.session.viewport.size;
		let stride = i32::from(size.width) * 4;
		let index = match self.buffers.iter().position(|frame| !frame.dirty && frame.size == size) {
			Some(index) => index,
			None => {
				let free = self.buffers.iter().position(|frame| !frame.dirty);
				if free.is_none() && self.buffers.len() == 2 {
					return Ok(());
				}
				let (buffer, _) = self.pool.create_buffer(size.width.into(), size.height.into(), stride, wl_shm::Format::Xrgb8888)?;
				let frame = SharedFrame { size, buffer, dirty: false };
				match free {
					Some(index) => {
						self.buffers[index] = frame;
						index
					}
					None => {
						self.buffers.push(frame);
						self.buffers.len() - 1
					}
				}
			}
		};
		let frame = &mut self.buffers[index];
		let (pixels, actual_stride, _) = self.session.draw()?;
		let bytes = self.pool.canvas(&frame.buffer).context("Wayland frame is still in use")?;
		ensure!(actual_stride == stride as usize && bytes.len() == pixels.len(), "Wayland frame stride mismatch");
		bytes.copy_from_slice(pixels);
		frame.dirty = true;
		if let Some(decorations) = &mut self.frame
			&& decorations.is_dirty()
			&& !decorations.is_hidden()
		{
			decorations.draw();
		}
		let surface = self.window.wl_surface();
		frame.buffer.attach_to(surface)?;
		if surface.version() >= 4 {
			surface.damage_buffer(0, 0, size.width.into(), size.height.into());
		} else {
			surface.damage(0, 0, size.width.into(), size.height.into());
		}
		surface.frame(qh, FrameCallbackData(surface.clone()));
		self.frame_pending = true;
		self.window.commit();
		Ok(())
	}

	fn input(&mut self, input: Input, qh: &QueueHandle<Self>, serial: u32) {
		if self.closed {
			return;
		}
		match self.session.input(input) {
			SelectionChange::Keep => {}
			SelectionChange::Clear => self.clear_selection(),
			SelectionChange::Publish => {
				let text = self.session.selected_text();
				if text.is_empty() {
					self.clear_selection();
				} else if let (Some(manager), Some(device)) = (&self.primary_manager, &self.primary_device) {
					let source = manager.create_selection_source(qh, TEXT_TYPES);
					source.set_selection(device, serial);
					self.selection = Some(Selection { source, bytes: Zeroizing::new(text.as_bytes().to_vec()), transfer: None });
				} else {
					self.session.app.status = Some("Wayland primary selection is unavailable on this compositor".to_owned());
					self.session.redraw = true;
				}
			}
		}
	}

	fn clear_selection(&mut self) {
		if let Some(owned) = self.selection.take() {
			// Dropping the source destroys only our offer, even if the
			// compositor has already switched to another client's source.
			drop(owned);
		}
	}

	fn pump_selection(&mut self) {
		let Some(selection) = &mut self.selection else { return };
		let Some((pipe, offset, deadline)) = &mut selection.transfer else { return };
		if Instant::now() >= *deadline {
			selection.transfer = None;
			return;
		}
		match pipe.write(&selection.bytes[*offset..]) {
			Ok(count) if count > 0 && *offset + count < selection.bytes.len() => *offset += count,
			Err(err) if matches!(err.kind(), io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock) => {}
			_ => selection.transfer = None,
		}
	}

	fn key(&mut self, key: xkb::Keycode, qh: &QueueHandle<Self>, serial: u32) {
		let Some(state) = &self.keymap else { return };
		let mut symbol = state.key_get_one_sym(key);
		if self.modifiers.contains(KeyModifiers::CONTROL) && symbol.key_char().is_some_and(|ch| !ch.is_ascii()) {
			let level = u32::from(self.modifiers.contains(KeyModifiers::SHIFT));
			if let Some(&latin) = state.get_keymap().key_get_syms_by_level(key, 0, level).first() {
				symbol = latin;
			}
		}
		if let Some(event) = translate(symbol, self.modifiers) {
			self.input(Input::Key(event), qh, serial);
		}
	}

	fn repeat_key(&mut self) {
		let Some((key, due)) = self.repeat else { return };
		if Instant::now() < due || self.repeat_rate <= 0 {
			return;
		}
		self.repeat = Some((key, Instant::now() + Duration::from_nanos(1_000_000_000 / self.repeat_rate as u64)));
		let Some(state) = &self.keymap else { return };
		if let Some(event) = translate(state.key_get_one_sym(key), self.modifiers)
			&& matches!(self.session.input(Input::Key(event)), SelectionChange::Clear)
		{
			self.clear_selection();
		}
	}
}

impl WindowHandler for Client {
	fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
		self.closed = true;
	}

	fn configure(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _: u32) {
		let mut size = self.session.size;
		if configure.decoration_mode == DecorationMode::Client && self.frame.is_none() {
			self.frame = FallbackFrame::new(&self.window, &self.shm, self.subcompositor.clone(), qh.clone()).ok();
		}
		let (mut x, mut y) = (0, 0);
		let mut outer = (u32::from(size.width), u32::from(size.height));
		if let Some(frame) = &mut self.frame {
			frame.set_hidden(configure.decoration_mode != DecorationMode::Client);
			if !frame.is_hidden() {
				frame.update_state(configure.state);
				frame.update_wm_capabilities(configure.capabilities);
				let (width, height) = configure.new_size;
				let (width, height) = match (width, height) {
					(Some(width), Some(height)) => frame.subtract_borders(width, height),
					other => other,
				};
				size = configured_size(size, width, height);
				frame.resize(NonZeroU32::new(size.width.into()).unwrap(), NonZeroU32::new(size.height.into()).unwrap());
				(x, y) = frame.location();
				outer = frame.add_borders(size.width.into(), size.height.into());
			}
		}
		if self.frame.as_ref().is_none_or(|frame| frame.is_hidden()) {
			size = configured_size(size, configure.new_size.0, configure.new_size.1);
			outer = (size.width.into(), size.height.into());
		}
		self.window.xdg_surface().set_window_geometry(x, y, outer.0 as i32, outer.1 as i32);
		self.session.resize(size);
		self.session.redraw = true;
		self.configured = true;
	}
}

fn configured_size(previous: Size, width: Option<NonZeroU32>, height: Option<NonZeroU32>) -> Size {
	Size::new(
		width.map_or(previous.width, |width| width.get().min(MAX_SIZE.width.into()) as u16),
		height.map_or(previous.height, |height| height.get().min(MAX_SIZE.height.into()) as u16),
	)
}

impl CompositorHandler for Client {
	fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
	fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
	fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
		if surface == self.window.wl_surface() {
			self.frame_pending = false;
		}
	}
	fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
	fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for Client {
	fn output_state(&mut self) -> &mut OutputState {
		&mut self.outputs
	}
	fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
	fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
	fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for Client {
	fn shm_state(&mut self) -> &mut Shm {
		&mut self.shm
	}
}

impl SeatHandler for Client {
	fn seat_state(&mut self) -> &mut SeatState {
		&mut self.seats
	}
	fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
	fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
		if self.seat.as_ref().is_some_and(|current| current != &seat) {
			return;
		}
		if self.seat.is_none() {
			self.primary_device = self.primary_manager.as_ref().map(|manager| manager.get_selection_device(qh, &seat));
			self.seat = Some(seat.clone());
		}
		match capability {
			Capability::Keyboard if self.keyboard.is_none() => self.keyboard = Some(seat.get_keyboard(qh, ())),
			Capability::Pointer if self.pointer.is_none() => {
				let surface = self.compositor.create_surface(qh);
				self.pointer = self.seats.get_pointer_with_theme::<Self, ()>(qh, &seat, self.shm.wl_shm(), surface, ThemeSpec::default()).ok();
			}
			_ => {}
		}
	}
	fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
		if self.seat.as_ref() != Some(&seat) {
			return;
		}
		match capability {
			Capability::Keyboard => {
				if let Some(keyboard) = self.keyboard.take()
					&& keyboard.version() >= 3
				{
					keyboard.release()
				}
				self.repeat = None;
				self.keymap = None;
				self.modifiers = KeyModifiers::NONE;
			}
			Capability::Pointer => self.pointer = None,
			_ => {}
		}
	}
	fn remove_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
		if self.seat.as_ref() == Some(&seat) {
			self.remove_capability(conn, qh, seat.clone(), Capability::Keyboard);
			self.remove_capability(conn, qh, seat, Capability::Pointer);
			self.seat = None;
			self.selection = None;
			self.primary_device = None;
			self.session.input(Input::FocusLost);
			self.session.selection_lost();
		}
	}
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
	fn event(client: &mut Self, keyboard: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
		use wl_keyboard::Event;
		if client.keyboard.as_ref() != Some(keyboard) {
			return;
		}
		match event {
			Event::Keymap { format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1), fd, size } => {
				let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
				// The compositor provides an XKB text keymap via this owned fd.
				client.keymap = unsafe { xkb::Keymap::new_from_fd(&context, fd, size as usize, xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS) }
					.ok()
					.flatten()
					.map(|map| xkb::State::new(&map));
				client.repeat = None;
			}
			Event::Key { serial, key, state: WEnum::Value(state), .. } => {
				let key = xkb::Keycode::new(key.saturating_add(8));
				match state {
					wl_keyboard::KeyState::Pressed => {
						client.key(key, qh, serial);
						if client.repeat_rate > 0 && client.keymap.as_ref().is_some_and(|state| state.get_keymap().key_repeats(key)) {
							client.repeat = Some((key, Instant::now() + Duration::from_millis(client.repeat_delay.max(0) as u64)));
						}
					}
					wl_keyboard::KeyState::Repeated => client.key(key, qh, serial),
					wl_keyboard::KeyState::Released if client.repeat.is_some_and(|(repeating, _)| repeating == key) => client.repeat = None,
					_ => {}
				}
			}
			Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
				if let Some(state) = &mut client.keymap {
					state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
					for (flag, name) in [
						(KeyModifiers::SHIFT, xkb::MOD_NAME_SHIFT),
						(KeyModifiers::CONTROL, xkb::MOD_NAME_CTRL),
						(KeyModifiers::ALT, xkb::MOD_NAME_ALT),
						(KeyModifiers::SUPER, xkb::MOD_NAME_LOGO),
					] {
						client.modifiers.set(flag, state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE));
					}
				}
			}
			Event::RepeatInfo { rate, delay } => {
				client.repeat_rate = rate.clamp(0, 1000);
				client.repeat_delay = delay;
				if rate <= 0 {
					client.repeat = None;
				}
			}
			Event::Leave { .. } => {
				client.repeat = None;
				client.session.input(Input::FocusLost);
			}
			_ => {}
		}
	}
}

impl PointerHandler for Client {
	fn pointer_frame(&mut self, conn: &Connection, qh: &QueueHandle<Self>, pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
		use PointerEventKind::*;
		for event in events {
			let (x, y) = event.position;
			if &event.surface == self.window.wl_surface() {
				let (x, y) = (x.round() as i16, y.round() as i16);
				match event.kind {
					Enter { .. } => {
						if let Some(pointer) = &self.pointer {
							let _ = pointer.set_cursor(conn, CursorIcon::Default);
						}
					}
					Motion { .. } => self.input(Input::Motion { x, y }, qh, 0),
					Press { button: LEFT_BUTTON, serial, time } => self.input(Input::Press { button: 1, x, y, time }, qh, serial),
					Release { button: LEFT_BUTTON, serial, .. } => self.input(Input::Release { x, y }, qh, serial),
					Press { serial, time, .. } => self.input(Input::Press { button: 2, x, y, time }, qh, serial),
					Axis { time, vertical, .. } => {
						let delta = if vertical.value120 != 0 { f64::from(vertical.value120) } else { vertical.absolute };
						if delta != 0.0 {
							let button = if delta < 0.0 { 4 } else { 5 };
							self.input(Input::Press { button, x, y, time }, qh, 0);
						}
					}
					_ => {}
				}
			} else if let Some(frame) = &mut self.frame {
				match event.kind {
					Enter { .. } | Motion { .. } => {
						if let Some(cursor) = frame.click_point_moved(Duration::ZERO, &event.surface.id(), x, y)
							&& let Some(pointer) = &self.pointer
						{
							let _ = pointer.set_cursor(conn, cursor);
						}
					}
					Leave { .. } => frame.click_point_left(),
					Press { button, serial, time } | Release { button, serial, time } => {
						let click = match button {
							LEFT_BUTTON => FrameClick::Normal,
							RIGHT_BUTTON => FrameClick::Alternate,
							_ => continue,
						};
						if let Some(action) = frame.on_click(Duration::from_millis(time.into()), click, matches!(event.kind, Press { .. }))
							&& let Some(data) = pointer.data::<PointerData<()>>()
						{
							self.frame_action(data.seat(), serial, action);
						}
					}
					_ => {}
				}
			}
		}
	}
}

impl Client {
	fn frame_action(&mut self, seat: &wl_seat::WlSeat, serial: u32, action: FrameAction) {
		match action {
			FrameAction::Close => self.closed = true,
			FrameAction::Move => self.window.move_(seat, serial),
			FrameAction::Minimize => self.window.set_minimized(),
			FrameAction::Maximize => self.window.set_maximized(),
			FrameAction::UnMaximize => self.window.unset_maximized(),
			FrameAction::ShowMenu(x, y) => self.window.show_window_menu(seat, serial, (x, y)),
			FrameAction::Resize(edge) => {
				let edge = match edge {
					ResizeEdge::Top => xdg_toplevel::ResizeEdge::Top,
					ResizeEdge::Bottom => xdg_toplevel::ResizeEdge::Bottom,
					ResizeEdge::Left => xdg_toplevel::ResizeEdge::Left,
					ResizeEdge::Right => xdg_toplevel::ResizeEdge::Right,
					ResizeEdge::TopLeft => xdg_toplevel::ResizeEdge::TopLeft,
					ResizeEdge::TopRight => xdg_toplevel::ResizeEdge::TopRight,
					ResizeEdge::BottomLeft => xdg_toplevel::ResizeEdge::BottomLeft,
					ResizeEdge::BottomRight => xdg_toplevel::ResizeEdge::BottomRight,
					_ => return,
				};
				self.window.resize(seat, serial, edge);
			}
			_ => {}
		}
	}
}

impl PrimarySelectionDeviceHandler for Client {
	fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1) {}
}

impl PrimarySelectionSourceHandler for Client {
	fn send_request(
		&mut self, _: &Connection, _: &QueueHandle<Self>, source: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1, mime: String, pipe: WritePipe,
	) {
		if let Some(selection) = &mut self.selection
			&& selection.source.inner() == source
			&& TEXT_TYPES.contains(&mime.as_str())
			&& selection.transfer.is_none()
			&& rustix::io::ioctl_fionbio(&pipe, true).is_ok()
		{
			selection.transfer = Some((File::from(std::os::fd::OwnedFd::from(pipe)), 0, Instant::now() + Duration::from_secs(2)));
			self.pump_selection();
		}
	}
	fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1) {
		if self.selection.as_ref().is_some_and(|selection| selection.source.inner() == source) {
			self.selection = None;
			self.session.selection_lost();
		}
	}
}

impl ProvidesRegistryState for Client {
	fn registry(&mut self) -> &mut RegistryState {
		&mut self.registry
	}
	sctk::registry_handlers![OutputState, SeatState];
}

sctk::delegate_registry!(Client);
sctk::delegate_dispatch2!(Client);

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wayland_configure_preserves_unspecified_axes_and_bounds_allocations() {
		let previous = Size::new(960, 624);
		assert_eq!(configured_size(previous, None, None), previous);
		assert_eq!(configured_size(previous, NonZeroU32::new(1200), None), Size::new(1200, 624));
		assert_eq!(configured_size(previous, NonZeroU32::new(u32::MAX), NonZeroU32::new(u32::MAX)), MAX_SIZE);
	}
}
