mod renderer;
mod translate;
mod window;

pub use window::{EguiWindow, Queue};

pub use egui;

use std::sync::atomic::{AtomicU32, Ordering};

/// Zoom der gesamten Oberfläche (1.0 = 100 %). Wird pro Frame gelesen;
/// Layout in Punkten bleibt gleich, nur Pixel pro Punkt ändern sich.
static UI_ZOOM: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32

pub fn set_ui_zoom(zoom: f32) {
    let z = if zoom.is_finite() { zoom.clamp(0.25, 4.0) } else { 1.0 };
    UI_ZOOM.store(z.to_bits(), Ordering::Relaxed);
}

pub fn ui_zoom() -> f32 {
    f32::from_bits(UI_ZOOM.load(Ordering::Relaxed))
}
pub use renderer::GraphicsConfig;
