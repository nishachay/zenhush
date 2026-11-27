//! Embedded Lucide SVG icons, rasterized lazily at 3x via resvg.

use std::collections::HashMap;

use egui::{Color32, ColorImage, TextureHandle};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IconKind {
    ChevronLeft,
    ChevronRight,
    Restart,
    Play,
    Pause,
    Volume,
    Mute,
    Lock,
    Unlock,
    Pin,
    Minimize,
    Maximize,
    Trash,
    Close,
    Sliders,
    FolderOpen,
    Timer,
}

impl IconKind {
    fn svg(self) -> &'static str {
        match self {
            IconKind::ChevronLeft => include_str!("../assets/icons/chevron-left.svg"),
            IconKind::ChevronRight => include_str!("../assets/icons/chevron-right.svg"),
            IconKind::Restart => include_str!("../assets/icons/rotate-ccw.svg"),
            IconKind::Play => include_str!("../assets/icons/play.svg"),
            IconKind::Pause => include_str!("../assets/icons/pause.svg"),
            IconKind::Volume => include_str!("../assets/icons/volume-2.svg"),
            IconKind::Mute => include_str!("../assets/icons/volume-x.svg"),
            IconKind::Lock => include_str!("../assets/icons/lock.svg"),
            IconKind::Unlock => include_str!("../assets/icons/unlock.svg"),
            IconKind::Pin => include_str!("../assets/icons/pin.svg"),
            IconKind::Minimize => include_str!("../assets/icons/minimize-2.svg"),
            IconKind::Maximize => include_str!("../assets/icons/maximize-2.svg"),
            IconKind::Trash => include_str!("../assets/icons/trash-2.svg"),
            IconKind::Close => include_str!("../assets/icons/x.svg"),
            IconKind::Sliders => include_str!("../assets/icons/sliders-horizontal.svg"),
            IconKind::FolderOpen => include_str!("../assets/icons/folder-open.svg"),
            IconKind::Timer => include_str!("../assets/icons/timer.svg"),
        }
    }

    fn filled(self) -> bool {
        matches!(self, IconKind::Play | IconKind::Pause)
    }
}

fn hex(color: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
}

fn render(kind: IconKind, color: Color32, px: u32) -> Option<ColorImage> {
    let hexcol = hex(color);
    let mut svg = kind.svg().replace("currentColor", &hexcol);
    if kind.filled() {
        for tag in ["<polygon", "<rect", "<path"] {
            let filled = format!("{} fill=\"{}\"", tag, hexcol);
            svg = svg.replace(tag, &filled);
        }
    }
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let render_px = px * 3;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(render_px, render_px)?;
    let scale = render_px as f32 / 24.0;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let size = [render_px as usize, render_px as usize];
    Some(ColorImage::from_rgba_premultiplied(size, pixmap.data()))
}

pub struct IconCache {
    cache: HashMap<(IconKind, [u8; 4], u32), TextureHandle>,
}

impl IconCache {
    pub fn new() -> Self {
        IconCache {
            cache: HashMap::new(),
        }
    }

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
            format!(
                "icon-{:?}-{:02X}{:02X}{:02X}-{}",
                kind,
                color.r(),
                color.g(),
                color.b(),
                px
            ),
            img,
            egui::TextureOptions::LINEAR,
        );
        self.cache.insert(key, tex.clone());
        Some(tex)
    }
}
