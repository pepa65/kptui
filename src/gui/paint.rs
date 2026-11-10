use cairo_sys as cairo;
use pango_sys as pango;
use std::ptr::{NonNull, null_mut};

use super::viewport::Viewport;

unsafe extern "C" {
	fn pango_cairo_create_context(cr: *mut cairo::cairo_t) -> *mut pango::PangoContext;
	fn pango_cairo_show_glyph_string(cr: *mut cairo::cairo_t, font: *mut pango::PangoFont, glyphs: *mut pango::PangoGlyphString);
}

pub struct Canvas {
	surface: NonNull<cairo::cairo_surface_t>,
	context: NonNull<cairo::cairo_t>,
	shaping: NonNull<pango::PangoContext>,
	height: usize,
	width: u16,
}

impl Canvas {
	pub(super) fn size(&self) -> ratatui::layout::Size {
		ratatui::layout::Size::new(self.width, self.height as u16)
	}

	pub fn new(viewport: &Viewport) -> Result<Self, &'static str> {
		// Cairo returns non-null error objects, which still need to be destroyed.
		unsafe {
			let surface = cairo::cairo_image_surface_create(cairo::FORMAT_RGB24, viewport.size.width.into(), viewport.size.height.into());
			let context = cairo::cairo_create(surface);
			if cairo::cairo_status(context) != cairo::STATUS_SUCCESS {
				cairo::cairo_destroy(context);
				cairo::cairo_surface_destroy(surface);
				return Err("could not create Cairo canvas");
			}
			cairo::cairo_translate(context, viewport.offset_x, viewport.offset_y);
			cairo::cairo_scale(context, viewport.scale, viewport.scale);
			let shaping = pango_cairo_create_context(context);
			let font = pango::pango_font_description_from_string(c"Monospace".as_ptr());
			pango::pango_font_description_set_absolute_size(font, f64::from(16 * pango::PANGO_SCALE));
			pango::pango_context_set_font_description(shaping, font);
			pango::pango_font_description_free(font);
			Ok(Self {
				surface: NonNull::new(surface).unwrap(),
				context: NonNull::new(context).unwrap(),
				shaping: NonNull::new(shaping).unwrap(),
				height: viewport.size.height.into(),
				width: viewport.size.width,
			})
		}
	}

	fn color(&self, rgb: u32) {
		let channel = |shift: u32| ((rgb >> shift) & 0xff) as f64 / 255.0;
		unsafe { cairo::cairo_set_source_rgb(self.context.as_ptr(), channel(16), channel(8), channel(0)) };
	}

	pub fn clear(&self, rgb: u32) {
		self.color(rgb);
		unsafe { cairo::cairo_paint(self.context.as_ptr()) };
	}

	pub fn rect(&self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
		self.color(rgb);
		unsafe {
			cairo::cairo_rectangle(self.context.as_ptr(), x.into(), y.into(), w.into(), h.into());
			cairo::cairo_fill(self.context.as_ptr());
		}
	}

	pub fn selection_overlay(&self, x: i32, y: i32, width: i32, height: i32, rgb: u32) {
		let [_, red, green, blue] = rgb.to_be_bytes();
		let context = self.context.as_ptr();
		unsafe {
			cairo::cairo_set_source_rgba(context, f64::from(red) / 255.0, f64::from(green) / 255.0, f64::from(blue) / 255.0, 0.45);
			cairo::cairo_rectangle(context, x.into(), y.into(), width.into(), height.into());
			cairo::cairo_fill(context);
		}
	}

	/// Shape borrowed UTF-8 directly, without storing the text in a PangoLayout.
	pub fn text(&self, x: i32, y: i32, width: i32, text: &str, color: u32) {
		let Ok(length) = i32::try_from(text.len()) else { return };
		self.color(color);
		unsafe {
			let text = text.as_ptr().cast::<std::ffi::c_char>();
			let direction = pango::pango_find_base_dir(text, length);
			let logical = pango::pango_itemize_with_base_dir(self.shaping.as_ptr(), direction, text, 0, length, null_mut(), null_mut());
			let visual = pango::pango_reorder_items(logical);
			let glyphs = pango::pango_glyph_string_new();
			let context = self.context.as_ptr();
			cairo::cairo_save(context);
			cairo::cairo_rectangle(context, x.into(), y.into(), width.into(), f64::from(super::CELL_HEIGHT));
			cairo::cairo_clip(context);

			let mut pen = f64::from(x);
			let mut node = visual;
			while !node.is_null() {
				let item = &*(*node).data.cast::<pango::PangoItem>();
				pango::pango_shape_with_flags(
					text.add(item.offset as usize),
					item.length,
					text,
					length,
					&item.analysis,
					glyphs,
					pango::PANGO_SHAPE_ROUND_POSITIONS,
				);
				if !item.analysis.font.is_null() {
					cairo::cairo_move_to(context, pen, f64::from(y + 17));
					pango_cairo_show_glyph_string(context, item.analysis.font, glyphs);
				}
				pen += f64::from(pango::pango_glyph_string_get_width(glyphs)) / f64::from(pango::PANGO_SCALE);
				node = (*node).next;
			}

			cairo::cairo_restore(context);
			pango::pango_glyph_string_free(glyphs);
			glib_sys::g_list_free(visual);
			let mut node = logical;
			while !node.is_null() {
				pango::pango_item_free((*node).data.cast());
				node = (*node).next;
			}
			glib_sys::g_list_free(logical);
		}
	}

	pub fn pixels(&mut self) -> Result<(&[u8], usize), &'static str> {
		// Flush pending drawing before borrowing the surface's bytes. The
		// mutable borrow keeps drawing and destruction out of that lifetime.
		unsafe {
			let surface = self.surface.as_ptr();
			cairo::cairo_surface_flush(surface);
			let data = cairo::cairo_image_surface_get_data(surface);
			if data.is_null() || cairo::cairo_status(self.context.as_ptr()) != cairo::STATUS_SUCCESS {
				return Err("could not read Cairo canvas");
			}
			let stride = cairo::cairo_image_surface_get_stride(surface) as usize;
			Ok((std::slice::from_raw_parts(data, stride * self.height), stride))
		}
	}
}

impl Drop for Canvas {
	fn drop(&mut self) {
		unsafe {
			// Erase our pixel buffer before Cairo releases it.
			cairo::cairo_surface_flush(self.surface.as_ptr());
			let stride = cairo::cairo_image_surface_get_stride(self.surface.as_ptr()) as usize;
			let data = cairo::cairo_image_surface_get_data(self.surface.as_ptr());
			if !data.is_null() {
				zeroize::Zeroize::zeroize(std::slice::from_raw_parts_mut(data, stride * self.height));
			}
			gobject_sys::g_object_unref(self.shaping.as_ptr().cast());
			cairo::cairo_destroy(self.context.as_ptr());
			cairo::cairo_surface_destroy(self.surface.as_ptr());
		}
	}
}
