//! The Lucide SVG icons the Python app uses, rasterized with resvg.
//! Same files, same recoloring: `currentColor` is replaced by the target
//! hex (Python `_icon_svg_bytes`), and play/pause are filled solid
//! (Python `_FILLED_KINDS`). Rendered at 3x for crisp edges, then
//! downscaled by egui.

use std::collections::HashMap;

use egui::{Color32, ColorImage, TextureHandle};

/// The six icons, same files as the Python app's `assets/icons/`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IconKind {
    ChevronLeft,
    ChevronRight,
    Restart,
    Gear,
    Play,
    Pause,
}

impl IconKind {
    /// The embedded SVG source (the Python app's file, byte for byte).
    fn svg(self) -> &'static str {
        match self {
            IconKind::ChevronLeft => include_str!("../assets/icons/chevron-left.svg"),
            IconKind::ChevronRight => include_str!("../assets/icons/chevron-right.svg"),
            IconKind::Restart => include_str!("../assets/icons/rotate-ccw.svg"),
            IconKind::Gear => include_str!("../assets/icons/settings.svg"),
            IconKind::Play => include_str!("../assets/icons/play.svg"),
            IconKind::Pause => include_str!("../assets/icons/pause.svg"),
        }
    }

    /// Play and pause are filled solid; the rest are stroked outlines.
    fn filled(self) -> bool {
        matches!(self, IconKind::Play | IconKind::Pause)
    }
}

fn hex(color: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
}

/// Rasterize one icon in one color at `px` pixels (the glyph fills the
/// square; the button around it supplies the padding, like Python's
/// `_pixmap` with its `icon_ratio`). Returns None on a bad SVG.
fn render(kind: IconKind, color: Color32, px: u32) -> Option<ColorImage> {
    let hexcol = hex(color);
    let mut svg = kind.svg().replace("currentColor", &hexcol);
    if kind.filled() {
        // Fill the path/rect geometry solid, keeping the stroke in the
        // same color so the rounded joins and caps survive.
        for tag in ["<polygon", "<rect", "<path"] {
            let filled = format!("{} fill=\"{}\"", tag, hexcol);
            svg = svg.replace(tag, &filled);
        }
    }
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    // The viewBox is 24x24; scale it to fill the px square. Render at
    // 3x and let egui downscale for crisp edges.
    let render_px = px * 3;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(render_px, render_px)?;
    let scale = render_px as f32 / 24.0;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let size = [render_px as usize, render_px as usize];
    Some(ColorImage::from_rgba_premultiplied(
        size,
        pixmap.data(),
    ))
}

/// Cache of rasterized icons, keyed by (icon, color RGBA, display px).
/// Re-rasterizing every frame would be slow; the theme only changes
/// color when the user switches appearance.
pub struct IconCache {
    cache: HashMap<(IconKind, [u8; 4], u32), TextureHandle>,
}

impl IconCache {
    pub fn new() -> Self {
        IconCache {
            cache: HashMap::new(),
        }
    }

    /// Get (or render) the texture for an icon in a color at a display
    /// size in points. The texture is 3x the display size.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        kind: IconKind,
        color: Color32,
        px: u32,
    ) -> Option<TextureHandle> {
        let key = (kind, color.to_array(), px);
        if let Some(t) = self.cache.get(&key) {
            return Some(t.clone());
        }
        let img = render(kind, color, px)?;
        let tex = ctx.load_texture(
            format!("icon-{:?}-{:02X}{:02X}{:02X}-{}", kind, color.r(), color.g(), color.b(), px),
            img,
            egui::TextureOptions::LINEAR,
        );
        self.cache.insert(key, tex.clone());
        Some(tex)
    }
}
