//! GUI: live preview, patch editor, fixture editor, QLC+ library, settings.
//! All texts are bilingual (German/English) via `t()` / `tf!`.

use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText};
use nih_plug_egui::{create_egui_editor, widgets::ParamSlider};

use crate::artnet;
use crate::config::{
    self, Capability, ChanKind, Effect, FixtureChannel, FixtureType, Level, MidiTarget, NoteFmt,
    PatchEntry, PatchedFixture, Settings, Show, TileInfo, UiPrefs, BLOCK_SIZE, UI_SCALES,
};
use crate::i18n::{self, t};
use crate::library;
use crate::preview;
use crate::qxf;
use crate::shared::{self, Shared, MAX_BUFS};
use crate::tf;
use crate::BridgeParams;

/// egui is small by default – "100 %" corresponds to zoom 1.25
pub const ZOOM_BASE: f32 = 1.25;

/// Zoom for a size preset (0.5 … 2.0)
pub fn zoom_for(scale: f32) -> f32 {
    scale * ZOOM_BASE
}

/// Window size (logical pixels) for a size preset and a layout size in points
pub fn window_size(scale: f32, points: (f32, f32)) -> (u32, u32) {
    let z = zoom_for(scale);
    ((points.0 * z).round() as u32, (points.1 * z).round() as u32)
}

/// Let the host resize the window too (Reaper: drag the FX window's edge)
pub fn allow_host_resize(params: &BridgeParams, scale: f32) {
    params
        .editor_state
        .set_host_resizable(true, window_size(scale, config::MIN_WINDOW));
}

const RED: Color32 = Color32::from_rgb(235, 90, 80);
const YELLOW: Color32 = Color32::from_rgb(235, 195, 70);
const GREEN: Color32 = Color32::from_rgb(100, 205, 120);
/// slightly lighter background for the stage
const STAGE_BG: Color32 = Color32::from_rgb(44, 44, 48);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Live,
    Patch,
    Fixtures,
    Library,
    Settings,
}

/// Working copy of a text fixture in the editor
#[derive(Clone)]
struct FxEdit {
    id: String,
    name: String,
    channels: Vec<FixtureChannel>,
    open_caps: Vec<bool>,
    /// state when loaded/saved – to detect unsaved changes
    base: Option<(String, Vec<FixtureChannel>)>,
}

impl FxEdit {
    fn dirty(&self) -> bool {
        match &self.base {
            Some((n, c)) => *n != self.name || *c != self.channels,
            None => true,
        }
    }
}

#[derive(Default)]
struct UiState {
    tab: Tab,
    initialized: bool,
    info: Option<(bool, String)>,
    dir_edit: String,

    rate_t: f64,
    rate_pkts: u64,
    rate: f32,

    show_ptr: usize,
    prefs: UiPrefs,
    /// prefs changed somewhere without access to params → save after the frame
    prefs_dirty: bool,
    /// window size (points) currently being dragged with the grip
    dragging: Option<(f32, f32)>,
    /// time when the window size last changed (saved once it is stable)
    size_changed_at: Option<f64>,
    /// strobe calculator: (device, channel) → wanted frequency in Hz
    strobe_target: std::collections::HashMap<(usize, usize), f32>,

    patch: Vec<PatchEntry>,
    patch_base: Vec<PatchEntry>,

    settings: Option<Settings>,
    settings_base: Option<Settings>,
    ip_text: String,

    fx_sel: Option<String>,
    fx_edit: Option<FxEdit>,
    new_fx_name: String,
    qxf_copy_mode: usize,

    lib_query: String,
    lib_last_query: Option<String>,
    lib_hits: Vec<(usize, Vec<usize>)>,
    lib_sel: Option<usize>,
    lib_parsed: Option<(usize, Result<FixtureType, String>)>,
    lib_mode: usize,
    lib_name: String,
    lib_universe: u16,
    lib_address: u16,
    lib_count: usize,
}

pub fn create(params: Arc<BridgeParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let egui_state = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        UiState::default(),
        |ctx: &egui::Context, _st: &mut UiState| {
            // Ubuntu-Light (default) lacks e.g. arrows → Hack as fallback font
            let mut fonts = egui::FontDefinitions::default();
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                list.push("Hack".to_owned());
            }
            ctx.set_fonts(fonts);
        },
        move |ctx: &egui::Context, setter: &ParamSetter, st: &mut UiState| {
            ctx.request_repaint();
            let now = ctx.input(|i| i.time);

            if !st.initialized {
                st.prefs = config::read_ui_prefs(&artnet::effective_dir(&params));
                i18n::set_english(st.prefs.english);
                request_window(&params, &st.prefs);
                st.dir_edit = artnet::effective_dir(&params).display().to_string();
                st.rate_t = now;
                st.rate_pkts = shared.packets_sent.load(Relaxed);
                st.lib_count = 1;
                st.initialized = true;
            }

            // packet rate instead of a counter
            if now - st.rate_t >= 1.0 {
                let p = shared.packets_sent.load(Relaxed);
                st.rate = (p.saturating_sub(st.rate_pkts)) as f32 / (now - st.rate_t) as f32;
                st.rate_pkts = p;
                st.rate_t = now;
            }

            let show: Option<Arc<Show>> = shared.show.lock().ok().and_then(|g| g.clone());
            if let Some(s) = &show {
                sync_with_show(st, s);
            }
            let master = master_value(&params);

            egui::CentralPanel::default().show(ctx, |ui| {
                header(ui, &params, &shared, setter, show.as_deref(), st);
                ui.add_space(4.0);
                tab_bar(ui, st);
                ui.separator();

                let Some(show) = show.as_deref() else {
                    ui.label(t("Lade Konfiguration …", "Loading configuration …"));
                    return;
                };
                if st.tab == Tab::Library {
                    // own scroll areas (manufacturer list and details)
                    library_tab(ui, show, &shared, st);
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("main-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match st.tab {
                        Tab::Live => live_tab(ui, show, &shared, master, st),
                        Tab::Patch => patch_tab(ui, show, &shared, st),
                        Tab::Fixtures => fixtures_tab(ui, show, &shared, st),
                        Tab::Library => {}
                        Tab::Settings => settings_tab(ui, show, &params, &shared, st),
                    });
            });

            resize_grip(ctx, &params, st);
            track_window_size(ctx, st, now);

            if st.prefs_dirty {
                st.prefs_dirty = false;
                save_prefs(&params, st);
            }
        },
    )
}

// ---------------------------------------------------------------------------
// General helpers
// ---------------------------------------------------------------------------

/// Framed block with a title – visually separates logical groups
fn section<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong());
            ui.separator();
            add(ui)
        })
        .inner
}

/// New configuration loaded → refresh working copies without unsaved changes.
fn sync_with_show(st: &mut UiState, show: &Arc<Show>) {
    let ptr = Arc::as_ptr(show) as usize;
    if ptr == st.show_ptr {
        return;
    }
    st.show_ptr = ptr;

    if st.patch == st.patch_base {
        st.patch = show.patch.clone();
    }
    st.patch_base = show.patch.clone();

    let new_settings = show.settings();
    if st.settings == st.settings_base {
        st.settings = Some(new_settings.clone());
        st.ip_text = new_settings.artnet_ip.to_string();
    }
    st.settings_base = Some(new_settings);

    if let Some(fx) = &mut st.fx_edit {
        if let Some(ti) = show.type_info(&fx.id).filter(|ti| !ti.is_qxf) {
            let loaded = (ti.ft.name.clone(), ti.ft.modes[0].channels.clone());
            if !fx.dirty() {
                fx.name = loaded.0.clone();
                fx.channels = loaded.1.clone();
                fx.open_caps.resize(fx.channels.len(), false);
            }
            fx.base = Some(loaded);
        }
    }
}

fn save_prefs(params: &BridgeParams, st: &UiState) {
    let _ = config::write_ui_prefs(&artnet::effective_dir(params), &st.prefs);
}

fn freq_text(hz: f32, bpm: bool) -> String {
    if bpm {
        format!("{:.0} BPM", hz * 60.0)
    } else if hz < 10.0 {
        format!("{hz:.1} Hz")
    } else {
        format!("{hz:.0} Hz")
    }
}

fn describe_cap(cap: &Capability, v: u8, bpm: bool) -> String {
    match cap.strobe_hz(v) {
        Some(hz) => format!("{} · {}", cap.label, freq_text(hz, bpm)),
        None => cap.label.clone(),
    }
}

/// Velocity that gives exactly this DMX value on note-on
fn velocity_for(dmx: u8) -> u8 {
    ((dmx as f32 * 127.0 / 255.0).round() as u8).clamp(1, 127)
}

/// Frequency input in the chosen unit; stored in Hz
fn freq_drag(ui: &mut egui::Ui, hz: &mut f32, bpm: bool) -> egui::Response {
    if bpm {
        let mut v = *hz * 60.0;
        let r = ui.add(egui::DragValue::new(&mut v).range(6.0..=6000.0).speed(1.0).suffix(" BPM"));
        if r.changed() {
            *hz = v / 60.0;
        }
        r
    } else {
        ui.add(egui::DragValue::new(hz).range(0.1..=100.0).speed(0.1).suffix(" Hz"))
    }
}

/// Black or white text, whichever is readable on this background
fn text_on(bg: Color32) -> Color32 {
    let lum = 0.2126 * bg.r() as f32 + 0.7152 * bg.g() as f32 + 0.0722 * bg.b() as f32;
    if lum > 150.0 {
        Color32::BLACK
    } else {
        Color32::from_rgb(240, 240, 240)
    }
}

fn master_value(params: &BridgeParams) -> f32 {
    if params.blackout.value() {
        0.0
    } else {
        params.master.value().clamp(0.0, 1.0)
    }
}

fn toggle(ui: &mut egui::Ui, setter: &ParamSetter, p: &BoolParam, label: &str) {
    let mut v = p.value();
    if ui.checkbox(&mut v, label).changed() {
        setter.begin_set_parameter(p);
        setter.set_parameter(p, v);
        setter.end_set_parameter(p);
    }
}

fn info_line(ui: &mut egui::Ui, st: &UiState) {
    if let Some((ok, txt)) = &st.info {
        ui.colored_label(if *ok { GREEN } else { RED }, txt.as_str());
    }
}

fn report(st: &mut UiState, shared: &Shared, r: Result<String, String>) {
    match r {
        Ok(msg) => {
            st.info = Some((true, msg));
            shared.reload_request.store(true, Relaxed);
        }
        Err(e) => st.info = Some((false, tf!("Fehler: {e}", "Error: {e}"))),
    }
}

fn open_path(p: &Path) {
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd).arg(p).spawn();
}

fn request_window(params: &BridgeParams, prefs: &UiPrefs) {
    request_window_points(params, prefs.scale, prefs.window);
}

fn request_window_points(params: &BridgeParams, scale: f32, points: (f32, f32)) {
    nih_plug_egui::set_ui_zoom(zoom_for(scale));
    allow_host_resize(params, scale);
    let want = window_size(scale, points);
    if params.editor_state.size() != want {
        params.editor_state.set_requested_size(want);
    }
}

/// FabFilter-style grip in the bottom-right corner: drag to resize the window,
/// the zoom (size preset) stays the same.
fn resize_grip(ctx: &egui::Context, params: &BridgeParams, st: &mut UiState) {
    let size = 16.0;
    egui::Area::new(egui::Id::new("resize-grip"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(0.0, 0.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::drag());
            let col = if resp.hovered() || resp.dragged() {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            let stroke = egui::Stroke::new(1.5, col);
            for i in 1..=3 {
                let o = i as f32 * 4.0;
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.right() - o, rect.bottom() - 1.0),
                        egui::pos2(rect.right() - 1.0, rect.bottom() - o),
                    ],
                    stroke,
                );
            }
            let resp = resp
                .on_hover_cursor(egui::CursorIcon::ResizeNwSe)
                .on_hover_text(t(
                    "Ziehen, um das Fenster zu vergrößern/verkleinern (Doppelklick: Standardgröße)",
                    "Drag to resize the window (double-click: default size)",
                ));
            if resp.double_clicked() {
                st.prefs.window = config::DEFAULT_WINDOW;
                st.prefs_dirty = true;
                request_window(params, &st.prefs);
                st.dragging = None;
                return;
            }
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    // pointer position in points = new layout size (plus half the grip)
                    let w = (p.x + size / 2.0).max(config::MIN_WINDOW.0);
                    let h = (p.y + size / 2.0).max(config::MIN_WINDOW.1);
                    st.dragging = Some((w, h));
                    request_window_points(params, st.prefs.scale, (w, h));
                }
            } else if let Some(sz) = st.dragging.take() {
                st.prefs.window = sz;
                st.prefs_dirty = true;
            }
        });
}

/// The host can resize the window as well – remember the size once it is stable.
fn track_window_size(ctx: &egui::Context, st: &mut UiState, now: f64) {
    if st.dragging.is_some() {
        return;
    }
    let r = ctx.screen_rect().size();
    let cur = (r.x.round(), r.y.round());
    let differs = (cur.0 - st.prefs.window.0).abs() > 2.0 || (cur.1 - st.prefs.window.1).abs() > 2.0;
    if !differs || cur.0 < config::MIN_WINDOW.0 - 2.0 || cur.1 < config::MIN_WINDOW.1 - 2.0 {
        st.size_changed_at = None;
        return;
    }
    match st.size_changed_at {
        None => st.size_changed_at = Some(now),
        Some(t0) if now - t0 > 0.8 => {
            st.prefs.window = cur;
            st.prefs_dirty = true;
            st.size_changed_at = None;
        }
        _ => {}
    }
}

fn note_range(fmt: NoteFmt, n1: u8, n2: u8) -> String {
    match fmt {
        NoteFmt::Number => format!("{n1}–{n2}"),
        NoteFmt::Name => format!("{}–{}", config::note_name(n1), config::note_name(n2)),
        NoteFmt::Both => format!(
            "{n1}–{n2} ({}–{})",
            config::note_name(n1),
            config::note_name(n2)
        ),
    }
}

fn midi_range_text(show: &Show, f: &PatchedFixture, fmt: NoteFmt) -> String {
    let last = f.channels.len() - 1;
    match (
        show.midi_for(f.universe, f.slot(0)),
        show.midi_for(f.universe, f.slot(last)),
    ) {
        (Some((c1, n1)), Some((c2, n2))) if c1 == c2 => {
            tf!("MIDI {} · Noten {}", "MIDI {} · notes {}", c1 + 1, note_range(fmt, n1, n2))
        }
        (Some((c1, n1)), Some((c2, n2))) => tf!(
            "MIDI {}/{} … MIDI {}/{} (über Blockgrenze)",
            "MIDI {}/{} … MIDI {}/{} (across blocks)",
            c1 + 1,
            config::note_text(n1, fmt),
            c2 + 1,
            config::note_text(n2, fmt)
        ),
        _ => t("MIDI: teilweise/keine Zuordnung", "MIDI: partly/not mapped").into(),
    }
}

// ---------------------------------------------------------------------------
// Header and tabs
// ---------------------------------------------------------------------------

fn header(
    ui: &mut egui::Ui,
    params: &BridgeParams,
    shared: &Shared,
    setter: &ParamSetter,
    show: Option<&Show>,
    st: &mut UiState,
) {
    ui.horizontal(|ui| {
        ui.heading("DMX Bridge");
        ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).weak());
        ui.add_space(16.0);
        ui.label("Master");
        ui.add(ParamSlider::for_param(&params.master, setter));
        toggle(ui, setter, &params.blackout, "Blackout");
        toggle(ui, setter, &params.output_enabled, t("Art-Net Ausgabe", "Art-Net output"));

        // size and language like in Waves plugins: top right
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut scale = st.prefs.scale;
            egui::ComboBox::from_id_salt("scale")
                .selected_text(format!("{:.0} %", scale * 100.0))
                .width(70.0)
                .show_ui(ui, |ui| {
                    for s in UI_SCALES {
                        ui.selectable_value(&mut scale, s, format!("{:.0} %", s * 100.0));
                    }
                });
            ui.label(t("Größe", "Size"));
            if scale != st.prefs.scale {
                st.prefs.scale = scale;
                save_prefs(params, st);
                request_window(params, &st.prefs);
            }

            ui.add_space(8.0);
            let mut english = st.prefs.english;
            egui::ComboBox::from_id_salt("lang")
                .selected_text(if english { "EN" } else { "DE" })
                .width(50.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut english, true, "English");
                    ui.selectable_value(&mut english, false, "Deutsch");
                });
            if english != st.prefs.english {
                st.prefs.english = english;
                i18n::set_english(english);
                save_prefs(params, st);
                // reload so that config messages appear in the new language
                shared.reload_request.store(true, Relaxed);
            }
        });
    });

    let target = show
        .map(|s| format!("{}:{}", s.artnet_ip, s.artnet_port))
        .unwrap_or_else(|| "–".into());
    let unis: Vec<String> = (0..MAX_BUFS)
        .filter(|&b| shared.buf_active[b].load(Relaxed))
        .map(|b| shared.buf_universe[b].load(Relaxed).to_string())
        .collect();
    let errors = shared.send_errors.load(Relaxed);
    ui.horizontal_wrapped(|ui| {
        let rate_col = if st.rate > 0.5 { GREEN } else { YELLOW };
        ui.label(tf!("Art-Net an {target}", "Art-Net to {target}"));
        ui.label("·");
        ui.colored_label(rate_col, tf!("{:.0} Pakete/s", "{:.0} packets/s", st.rate));
        ui.label("·");
        ui.label(tf!("Universen [{}]", "Universes [{}]", unis.join(", ")));
        if errors > 0 {
            ui.label("·");
            ui.colored_label(RED, tf!("{errors} Sendefehler", "{errors} send errors"));
        }
        if let Some(s) = show {
            let n = s.error_count();
            if n > 0 {
                ui.label("·");
                ui.colored_label(RED, tf!("{n} Konfig-Fehler (siehe Live)", "{n} config errors (see Live)"));
            }
        }
    });

    if !params.output_enabled.value() {
        ui.colored_label(
            YELLOW,
            t("Ausgabe deaktiviert – es wird nichts gesendet.", "Output disabled – nothing is sent."),
        );
    } else if params.blackout.value() {
        ui.colored_label(RED, t("BLACKOUT aktiv", "BLACKOUT active"));
    }
    if let Some(e) = shared.net_error.lock().ok().and_then(|g| g.clone()) {
        ui.colored_label(RED, tf!("Netzwerk: {e}", "Network: {e}"));
    }
    ui.label(RichText::new(last_midi_text(shared, show, st.prefs.notes)).monospace().small());
}

fn last_midi_text(shared: &Shared, show: Option<&Show>, fmt: NoteFmt) -> String {
    if shared.event_count.load(Relaxed) == 0 {
        return t("Noch kein MIDI empfangen.", "No MIDI received yet.").into();
    }
    let ev = shared.last_event.load(Relaxed);
    let kind = ev >> 24;
    let ch = ((ev >> 16) & 0x0F) as u8;
    let n = ((ev >> 8) & 0xFF) as u8;
    let v = (ev & 0xFF) as u8;
    let kind_s = match kind {
        shared::EV_NOTE_ON => "Note On",
        shared::EV_NOTE_OFF => "Note Off",
        shared::EV_PRESSURE => "Poly-AT",
        shared::EV_CC => "CC",
        shared::EV_RELEASE => "All Notes Off",
        _ => "?",
    };
    if kind == shared::EV_RELEASE {
        return tf!("Letztes MIDI: {kind_s} · Kanal {}", "Last MIDI: {kind_s} · channel {}", ch + 1);
    }
    let dest = show
        .and_then(|s| {
            let m = s.midi[ch as usize]?;
            let slot = m.first_slot() + n as usize;
            Some(match s.lookup(m.universe, slot) {
                Some((f, c)) => format!("U{} DMX {} = {}: {}", m.universe, slot + 1, f.name, c.label),
                None => tf!(
                    "U{} DMX {} (nicht gepatcht)",
                    "U{} DMX {} (not patched)",
                    m.universe,
                    slot + 1
                ),
            })
        })
        .unwrap_or_else(|| t("MIDI-Kanal nicht zugeordnet", "MIDI channel not mapped").into());
    let what = if kind == shared::EV_CC {
        format!("CC {n}")
    } else {
        format!("{} {}", t("Note", "note"), config::note_text(n, fmt))
    };
    tf!(
        "Letztes MIDI: {kind_s} · Kanal {} · {what} → {v}  ·  {dest}",
        "Last MIDI: {kind_s} · channel {} · {what} → {v}  ·  {dest}",
        ch + 1
    )
}

fn tab_bar(ui: &mut egui::Ui, st: &mut UiState) {
    let patch_dirty = st.patch != st.patch_base;
    let settings_dirty = st.settings != st.settings_base;
    let fx_dirty = st.fx_edit.as_ref().is_some_and(|f| f.dirty());
    let tabs = [
        (Tab::Live, "Live", false),
        (Tab::Patch, "Patch", patch_dirty),
        (Tab::Fixtures, t("Eigene Fixtures", "Custom Fixtures"), fx_dirty),
        (Tab::Library, t("Fixture-Bibliothek", "Fixture Library"), false),
        (Tab::Settings, t("Einstellungen", "Settings"), settings_dirty),
    ];
    ui.horizontal_wrapped(|ui| {
        for (tab, label, dirty) in tabs {
            let text = if dirty { format!("{label} •") } else { label.to_string() };
            // real buttons: the inactive ones keep their grey frame
            if ui
                .add(egui::Button::new(RichText::new(text).strong()).selected(st.tab == tab))
                .clicked()
            {
                st.tab = tab;
                st.info = None;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Live
// ---------------------------------------------------------------------------

fn live_tab(ui: &mut egui::Ui, show: &Show, shared: &Shared, master: f32, st: &mut UiState) {
    let bpm = st.prefs.bpm;
    let fmt = st.prefs.notes;

    let problems: Vec<_> = show.messages.iter().filter(|m| m.level != Level::Info).collect();
    if !problems.is_empty() {
        section(ui, t("Hinweise zur Konfiguration", "Configuration notes"), |ui| {
            for m in problems {
                match m.level {
                    Level::Error => ui.colored_label(RED, tf!("Fehler: {}", "Error: {}", m.text)),
                    _ => ui.colored_label(YELLOW, tf!("Hinweis: {}", "Note: {}", m.text)),
                };
            }
        });
        ui.add_space(6.0);
    }

    if show.fixtures.is_empty() {
        ui.label(t(
            "Keine Geräte gepatcht – im Tab „Patch“ oder „Fixture-Bibliothek“ hinzufügen.",
            "No devices patched – add some in the “Patch” or “Fixture Library” tab.",
        ));
        return;
    }
    let time = ui.input(|i| i.time);

    let all_vals: Vec<Vec<u8>> = show
        .fixtures
        .iter()
        .map(|f| {
            let buf = shared.buffer_for_universe(f.universe);
            (0..f.channels.len())
                .map(|c| buf.map_or(0, |b| shared.output_value(b, f.slot(c), master)))
                .collect()
        })
        .collect();

    // ---- stage ----
    let mut tile = st.prefs.tile;
    section(ui, t("Bühne", "Stage"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(t("Info unter den Kacheln:", "Info below tiles:")).small());
            egui::ComboBox::from_id_salt("tileinfo")
                .selected_text(tile_info_label(tile))
                .width(110.0)
                .show_ui(ui, |ui| {
                    for ti in [TileInfo::Nothing, TileInfo::Address, TileInfo::Dimmer, TileInfo::Midi] {
                        ui.selectable_value(&mut tile, ti, tile_info_label(ti));
                    }
                });
        });
        ui.add_space(4.0);
        egui::Frame::NONE.fill(STAGE_BG).corner_radius(6.0).inner_margin(8.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                for (f, vals) in show.fixtures.iter().zip(&all_vals) {
                    stage_tile(ui, show, f, vals, time, bpm, fmt, tile);
                }
            });
        });
    });
    if tile != st.prefs.tile {
        st.prefs.tile = tile;
        st.prefs_dirty = true;
    }

    ui.add_space(8.0);

    // ---- faders ----
    section(ui, "Fader", |ui| {
        ui.label(
            RichText::new(t(
                "Fader setzen den Wert direkt – das nächste MIDI-Event auf dem Kanal überschreibt ihn.",
                "Faders set the value directly – the next MIDI event on that channel overrides it.",
            ))
            .weak()
            .small(),
        );
        for (i, (f, vals)) in show.fixtures.iter().zip(&all_vals).enumerate() {
            fader_block(ui, show, shared, st, i, f, vals, bpm, fmt);
        }
    });
}

fn tile_info_label(ti: TileInfo) -> &'static str {
    match ti {
        TileInfo::Nothing => t("nichts", "nothing"),
        TileInfo::Address => t("DMX-Adresse", "DMX address"),
        TileInfo::Dimmer => "Dimmer %",
        TileInfo::Midi => t("MIDI-Noten", "MIDI notes"),
    }
}

#[allow(clippy::too_many_arguments)]
fn stage_tile(
    ui: &mut egui::Ui,
    show: &Show,
    f: &PatchedFixture,
    vals: &[u8],
    time: f64,
    bpm: bool,
    fmt: NoteFmt,
    tile: TileInfo,
) {
    let p = preview::compute(f, vals, time);
    ui.vertical(|ui| {
        ui.set_max_width(80.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(72.0, 44.0), egui::Sense::hover());
        let fill = if p.lit {
            Color32::from_rgb(p.rgb[0], p.rgb[1], p.rgb[2])
        } else {
            Color32::from_rgb(18, 18, 18)
        };
        ui.painter().rect_filled(rect, 4.0, fill);
        if let preview::Shutter::Blink { hz, .. } = p.shutter {
            // high contrast: black on bright, white on dark – also while flashing white
            ui.painter().text(
                rect.right_bottom() - egui::vec2(3.0, 2.0),
                egui::Align2::RIGHT_BOTTOM,
                freq_text(hz, bpm),
                egui::FontId::proportional(11.0),
                text_on(fill),
            );
        }
        resp.on_hover_text(format!(
            "{}\n{} · {}\nU{} · DMX {}\n{}",
            f.name,
            f.type_name,
            f.mode_name,
            f.universe,
            f.address,
            midi_range_text(show, f, fmt)
        ));
        ui.label(RichText::new(f.name.as_str()).small());
        let extra = match tile {
            TileInfo::Nothing => None,
            TileInfo::Address => Some(format!("U{} · {}", f.universe, f.address)),
            TileInfo::Dimmer => f
                .channels
                .iter()
                .position(|c| c.kind == ChanKind::Dimmer)
                .map(|c| format!("{:.0} %", vals[c] as f32 / 2.55)),
            TileInfo::Midi => show.midi_for(f.universe, f.slot(0)).map(|(mc, n1)| {
                let n2 = show
                    .midi_for(f.universe, f.slot(f.channels.len() - 1))
                    .map_or(n1, |(_, n)| n);
                format!("M{} · {}", mc + 1, note_range(fmt, n1, n2))
            }),
        };
        if let Some(x) = extra {
            ui.label(RichText::new(x).small().weak());
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn fader_block(
    ui: &mut egui::Ui,
    show: &Show,
    shared: &Shared,
    st: &mut UiState,
    i: usize,
    f: &PatchedFixture,
    vals: &[u8],
    bpm: bool,
    fmt: NoteFmt,
) {
    let buf = shared.buffer_for_universe(f.universe);
    let title = format!(
        "{}   ·   {} ({})   ·   U{} DMX {}–{}   ·   {}",
        f.name,
        f.type_name,
        f.mode_name,
        f.universe,
        f.address,
        f.address as usize + f.channels.len() - 1,
        midi_range_text(show, f, fmt)
    );
    ui.push_id(i, |ui| {
        egui::CollapsingHeader::new(title)
            .default_open(show.fixtures.len() <= 2)
            .show(ui, |ui| {
                if buf.is_none() {
                    ui.colored_label(
                        YELLOW,
                        t(
                            "Universum ist keinem MIDI-Kanal zugeordnet – wird nicht gesendet.",
                            "Universe is not mapped to a MIDI channel – not sent.",
                        ),
                    );
                }
                egui::Grid::new("chans").striped(true).show(ui, |ui| {
                    for (c, ch) in f.channels.iter().enumerate() {
                        let slot = f.slot(c);
                        ui.label(format!("{}", slot + 1));
                        ui.label(ch.label.as_str());
                        ui.label(RichText::new(ch.kind.label()).weak());
                        match buf {
                            Some(b) => {
                                let mut v = shared.raw_value(b, slot);
                                if ui.add(egui::Slider::new(&mut v, 0..=255u8)).changed() {
                                    shared.set_manual(b, slot, v);
                                }
                            }
                            None => {
                                ui.label("–");
                            }
                        }
                        let out = vals[c];
                        ui.label(ch.cap_at(out).map(|cap| describe_cap(cap, out, bpm)).unwrap_or_default());
                        ui.label(
                            RichText::new(match show.midi_for(f.universe, slot) {
                                Some((mc, n)) => format!(
                                    "MIDI {} · {} {}",
                                    mc + 1,
                                    t("Note", "note"),
                                    config::note_text(n, fmt)
                                ),
                                None => t("keine MIDI-Zuordnung", "no MIDI mapping").into(),
                            })
                            .weak(),
                        );
                        ui.end_row();

                        // strobe calculator: wanted frequency → DMX value / velocity
                        let strobe_caps: Vec<&Capability> = ch
                            .caps
                            .iter()
                            .filter(|k| matches!(k.effect, Effect::Strobe { .. }))
                            .collect();
                        if !strobe_caps.is_empty() {
                            let target = st.strobe_target.entry((i, c)).or_insert(8.0);
                            ui.label("");
                            ui.label(RichText::new(t("Wunschfrequenz", "Target frequency")).weak());
                            ui.horizontal(|ui| {
                                freq_drag(ui, target, bpm);
                                ui.label(RichText::new(format!("= {}", freq_text(*target, !bpm))).weak());
                            });
                            let hit = strobe_caps.iter().find_map(|k| k.value_for_hz(*target));
                            match hit {
                                Some(dmx) => {
                                    let vel = velocity_for(dmx);
                                    // velocity has only 128 steps → show the real frequency
                                    let dmx_from_vel = shared::to_dmx(vel as f32 / 127.0);
                                    let real = strobe_caps
                                        .iter()
                                        .find_map(|k| {
                                            k.strobe_hz(dmx_from_vel).filter(|_| k.cap_contains(dmx_from_vel))
                                        })
                                        .map(|hz| format!(" (≈ {})", freq_text(hz, bpm)))
                                        .unwrap_or_default();
                                    ui.label(format!("→ DMX {dmx}"));
                                    ui.label(RichText::new(format!("Velocity {vel}{real}")).strong());
                                    if let Some(b) = buf {
                                        if ui.button(t("setzen", "set")).clicked() {
                                            shared.set_manual(b, slot, dmx);
                                        }
                                    } else {
                                        ui.label("");
                                    }
                                }
                                None => {
                                    let (lo, hi) = strobe_caps.iter().fold((f32::MAX, 0f32), |(lo, hi), k| {
                                        match k.effect {
                                            Effect::Strobe { from, to, .. } => {
                                                (lo.min(from.min(to)), hi.max(from.max(to)))
                                            }
                                            _ => (lo, hi),
                                        }
                                    });
                                    ui.colored_label(
                                        YELLOW,
                                        tf!(
                                            "außerhalb ({}–{})",
                                            "out of range ({}–{})",
                                            freq_text(lo, bpm),
                                            freq_text(hi, bpm)
                                        ),
                                    );
                                    ui.label("");
                                    ui.label("");
                                }
                            }
                            ui.end_row();
                        }
                    }
                });
            });
    });
}

// ---------------------------------------------------------------------------
// Patch
// ---------------------------------------------------------------------------

enum RowAction {
    Up(usize),
    Down(usize),
    Remove(usize),
}

fn patch_tab(ui: &mut egui::Ui, show: &Show, shared: &Shared, st: &mut UiState) {
    let dirty = st.patch != st.patch_base;
    ui.horizontal_wrapped(|ui| {
        if ui.button(t("+ Gerät", "+ Device")).clicked() {
            if let Some(ti) = show.types.first() {
                let len = ti.ft.modes.first().map_or(1, |m| m.channels.len());
                let address = show.next_free_address(&st.patch, 0, len).unwrap_or(1);
                st.patch.push(PatchEntry {
                    name: ti.ft.name.clone(),
                    fixture: ti.ft.id.clone(),
                    universe: 0,
                    address,
                    mode: None,
                    count: 1,
                    spacing: None,
                });
            }
        }
        ui.separator();
        if ui.add_enabled(dirty, egui::Button::new(t("Speichern", "Save"))).clicked() {
            let r = config::write_patch(&show.dir, &st.patch)
                .map(|_| t("patch.txt gespeichert", "patch.txt saved").to_string());
            if r.is_ok() {
                st.patch_base = st.patch.clone();
            }
            report(st, shared, r);
        }
        if ui.add_enabled(dirty, egui::Button::new(t("Verwerfen", "Discard"))).clicked() {
            st.patch = show.patch.clone();
            st.patch_base = show.patch.clone();
        }
        if dirty {
            ui.colored_label(YELLOW, t("ungespeicherte Änderungen", "unsaved changes"));
        }
    });
    info_line(ui, st);
    ui.label(
        RichText::new(t(
            "Neue Gerätetypen findest du im Tab „Fixture-Bibliothek“ oder legst sie unter „Eigene Fixtures“ an.",
            "New device types: see the “Fixture Library” tab or create them under “Custom Fixtures”.",
        ))
        .weak()
        .small(),
    );
    ui.add_space(4.0);

    section(ui, t("Geräte", "Devices"), |ui| {
        if st.patch.is_empty() {
            ui.label(t("Noch keine Geräte gepatcht.", "No devices patched yet."));
        }

        let mut action: Option<RowAction> = None;
        let n = st.patch.len();
        egui::Grid::new("patchgrid")
            .striped(true)
            .num_columns(9)
            .show(ui, |ui| {
                for h in [
                    t("Name", "Name"),
                    "Fixture",
                    t("Modus", "Mode"),
                    t("Univ.", "Univ."),
                    t("Adresse", "Address"),
                    t("Anzahl", "Count"),
                    t("Kanäle", "Channels"),
                    "DMX",
                    "",
                ] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();

                for i in 0..n {
                    let e = &mut st.patch[i];
                    let name_resp = ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(220.0));
                    name_resp.on_hover_text(e.name.clone());

                    let tinfo = show.type_info(&e.fixture);
                    let fx_label = tinfo
                        .map(|ti| ti.ft.name.clone())
                        .unwrap_or_else(|| tf!("‹fehlt: {}›", "‹missing: {}›", e.fixture));
                    egui::ComboBox::from_id_salt(("fx", i))
                        .selected_text(fx_label)
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for ti in &show.types {
                                let src = if ti.is_qxf { "QLC+" } else { "txt" };
                                if ui
                                    .selectable_label(e.fixture == ti.ft.id, format!("{}  ({src})", ti.ft.name))
                                    .clicked()
                                {
                                    e.fixture = ti.ft.id.clone();
                                    e.mode = None;
                                }
                            }
                        });

                    match tinfo.filter(|ti| ti.ft.modes.len() > 1) {
                        Some(ti) => {
                            let cur = config::pick_mode(&ti.ft, e.mode.as_deref())
                                .map(|m| m.name.clone())
                                .unwrap_or_else(|| "?".into());
                            egui::ComboBox::from_id_salt(("mode", i))
                                .selected_text(cur.clone())
                                .show_ui(ui, |ui| {
                                    for m in &ti.ft.modes {
                                        if ui
                                            .selectable_label(
                                                cur == m.name,
                                                tf!("{} ({} Kan.)", "{} ({} ch.)", m.name, m.channels.len()),
                                            )
                                            .clicked()
                                        {
                                            e.mode = Some(m.name.clone());
                                        }
                                    }
                                });
                        }
                        None => {
                            ui.label("–");
                        }
                    }

                    ui.add(egui::DragValue::new(&mut e.universe).range(0..=config::MAX_UNIVERSE));
                    ui.add(egui::DragValue::new(&mut e.address).range(1..=512));
                    ui.add(egui::DragValue::new(&mut e.count).range(1..=512));

                    let len = show.entry_channels(e);
                    ui.label(len.to_string());
                    let last = e.address as usize + len.max(1) - 1;
                    let dmx = if e.count > 1 {
                        format!("{}–{} (×{})", e.address, last, e.count)
                    } else {
                        format!("{}–{}", e.address, last)
                    };
                    ui.label(dmx);

                    ui.horizontal(|ui| {
                        if ui.add_enabled(i > 0, egui::Button::new("⏶")).clicked() {
                            action = Some(RowAction::Up(i));
                        }
                        if ui.add_enabled(i + 1 < n, egui::Button::new("⏷")).clicked() {
                            action = Some(RowAction::Down(i));
                        }
                        if ui.button(t("Entfernen", "Remove")).clicked() {
                            action = Some(RowAction::Remove(i));
                        }
                    });
                    ui.end_row();
                }
            });

        match action {
            Some(RowAction::Up(i)) => st.patch.swap(i, i - 1),
            Some(RowAction::Down(i)) => st.patch.swap(i, i + 1),
            Some(RowAction::Remove(i)) => {
                st.patch.remove(i);
            }
            None => {}
        }
    });

    let msgs: Vec<_> = show
        .messages
        .iter()
        .filter(|m| {
            m.level != Level::Info
                && (m.text.starts_with("patch.txt")
                    || m.text.contains("Überschneidung")
                    || m.text.contains("Overlap")
                    || m.text.contains("MIDI"))
        })
        .collect();
    if !msgs.is_empty() {
        ui.add_space(6.0);
        section(ui, t("Stand der gespeicherten Datei", "Saved file status"), |ui| {
            for m in msgs {
                let col = if m.level == Level::Error { RED } else { YELLOW };
                ui.colored_label(col, m.text.as_str());
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn load_fx_edit(show: &Show, id: &str) -> Option<FxEdit> {
    let ti = show.type_info(id).filter(|ti| !ti.is_qxf)?;
    let channels = ti.ft.modes[0].channels.clone();
    Some(FxEdit {
        id: id.to_string(),
        name: ti.ft.name.clone(),
        open_caps: vec![false; channels.len()],
        base: Some((ti.ft.name.clone(), channels.clone())),
        channels,
    })
}

fn fixtures_tab(ui: &mut egui::Ui, show: &Show, shared: &Shared, st: &mut UiState) {
    let bpm = st.prefs.bpm;
    section(ui, t("Fixture wählen oder anlegen", "Choose or create a fixture"), |ui| {
        ui.horizontal_wrapped(|ui| {
            let sel_label = st
                .fx_sel
                .as_ref()
                .and_then(|id| {
                    show.type_info(id)
                        .map(|ti| ti.ft.name.clone())
                        .or_else(|| Some(tf!("{id} (neu)", "{id} (new)")))
                })
                .unwrap_or_else(|| t("– auswählen –", "– select –").into());
            let mut picked: Option<String> = None;
            egui::ComboBox::from_id_salt("fxsel")
                .selected_text(sel_label)
                .width(300.0)
                .show_ui(ui, |ui| {
                    for ti in &show.types {
                        let src = if ti.is_qxf { "QLC+" } else { "txt" };
                        if ui
                            .selectable_label(
                                st.fx_sel.as_deref() == Some(ti.ft.id.as_str()),
                                format!("{}  ({src})", ti.ft.name),
                            )
                            .clicked()
                        {
                            picked = Some(ti.ft.id.clone());
                        }
                    }
                });
            if let Some(id) = picked {
                st.fx_edit = load_fx_edit(show, &id);
                st.fx_sel = Some(id);
                st.qxf_copy_mode = 0;
                st.info = None;
            }

            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut st.new_fx_name)
                    .hint_text(t("Name für neues Fixture", "Name for a new fixture"))
                    .desired_width(200.0),
            );
            if ui
                .add_enabled(!st.new_fx_name.trim().is_empty(), egui::Button::new(t("Neu anlegen", "Create")))
                .clicked()
            {
                let name = st.new_fx_name.trim().to_string();
                let id = config::unique_fixture_id(&show.dir, &name);
                st.fx_edit = Some(FxEdit {
                    id: id.clone(),
                    name,
                    channels: vec![FixtureChannel {
                        kind: ChanKind::Dimmer,
                        label: "Dimmer".into(),
                        rest: 0,
                        caps: Vec::new(),
                    }],
                    open_caps: vec![false],
                    base: None,
                });
                st.fx_sel = Some(id);
                st.new_fx_name.clear();
            }
            ui.separator();
            if ui
                .button(t("Generic-Vorlagen hinzufügen", "Add generic templates"))
                .on_hover_text(t(
                    "Legt fehlende Vorlagen (Generic Dimmer, PAR, Wall Washer, Moving Head) in fixtures/ an – \
                     überschreibt nichts.",
                    "Creates missing templates (generic dimmer, PAR, wall washer, moving head) in fixtures/ – \
                     never overwrites anything.",
                ))
                .clicked()
            {
                let r = config::add_generic_templates(&show.dir).map(|n| match n {
                    0 => t("Alle Vorlagen sind schon vorhanden.", "All templates already exist.").to_string(),
                    n => tf!("{n} Vorlage(n) angelegt.", "{n} template(s) created."),
                });
                report(st, shared, r);
            }
        });
        info_line(ui, st);
    });
    ui.add_space(6.0);

    // QLC+ fixture: view only, editable as a text copy
    if st.fx_edit.is_none() {
        if let Some(ti) = st.fx_sel.as_deref().and_then(|id| show.type_info(id)) {
            if ti.is_qxf {
                section(ui, ti.ft.name.as_str(), |ui| {
                    ui.label(tf!(
                        "QLC+-Definition (fixtures/{}). Zum Bearbeiten eine Textkopie eines Modus anlegen:",
                        "QLC+ definition (fixtures/{}). To edit it, create a text copy of one mode:",
                        ti.file
                    ));
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("qxfmode")
                            .selected_text(
                                ti.ft.modes
                                    .get(st.qxf_copy_mode)
                                    .map(|m| m.name.clone())
                                    .unwrap_or_default(),
                            )
                            .show_ui(ui, |ui| {
                                for (i, m) in ti.ft.modes.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut st.qxf_copy_mode,
                                        i,
                                        tf!("{} ({} Kan.)", "{} ({} ch.)", m.name, m.channels.len()),
                                    );
                                }
                            });
                        if ui
                            .button(t("Als bearbeitbare Textdatei kopieren", "Copy as editable text file"))
                            .clicked()
                        {
                            if let Some(m) = ti.ft.modes.get(st.qxf_copy_mode) {
                                let id = config::unique_fixture_id(&show.dir, &format!("{}_custom", ti.ft.id));
                                st.fx_edit = Some(FxEdit {
                                    id: id.clone(),
                                    name: format!("{} ({})", ti.ft.name, m.name),
                                    open_caps: vec![false; m.channels.len()],
                                    channels: m.channels.clone(),
                                    base: None,
                                });
                                st.fx_sel = Some(id);
                            }
                        }
                    });
                    if let Some(m) = ti.ft.modes.get(st.qxf_copy_mode) {
                        ui.add_space(4.0);
                        channel_overview(ui, &m.channels);
                    }
                });
                return;
            }
        }
        ui.label(t(
            "Wähle oben ein Fixture aus oder lege ein neues an.",
            "Choose a fixture above or create a new one.",
        ));
        return;
    }

    let Some(fx) = st.fx_edit.as_mut() else { return };
    let dirty = fx.dirty();
    let mut save = false;
    let mut discard = false;

    section(ui, t("Fixture bearbeiten", "Edit fixture"), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Name:", "Name:"));
            ui.add(egui::TextEdit::singleline(&mut fx.name).desired_width(260.0));
            ui.label(RichText::new(tf!("Datei: fixtures/{}.txt", "File: fixtures/{}.txt", fx.id)).weak());
        });
        ui.horizontal_wrapped(|ui| {
            save = ui.add_enabled(dirty, egui::Button::new(t("Speichern", "Save"))).clicked();
            discard = ui
                .add_enabled(dirty && fx.base.is_some(), egui::Button::new(t("Verwerfen", "Discard")))
                .clicked();
            if dirty {
                ui.colored_label(YELLOW, t("ungespeicherte Änderungen", "unsaved changes"));
            }
            ui.separator();
            let all_open = !fx.open_caps.is_empty() && fx.open_caps.iter().all(|o| *o);
            if ui
                .button(if all_open {
                    t("Alle Bereiche zuklappen", "Collapse all ranges")
                } else {
                    t("Alle Bereiche aufklappen", "Expand all ranges")
                })
                .clicked()
            {
                for o in fx.open_caps.iter_mut() {
                    *o = !all_open;
                }
            }
        });
    });
    ui.add_space(6.0);

    fx.open_caps.resize(fx.channels.len(), false);
    let mut action: Option<RowAction> = None;
    let n = fx.channels.len();
    section(ui, t("Kanäle", "Channels"), |ui| {
        for c in 0..n {
            ui.push_id(("ch", c), |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let ch = &mut fx.channels[c];
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("{:>3}", c + 1)).monospace().strong());
                        egui::ComboBox::from_id_salt("kind")
                            .selected_text(ch.kind.label())
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for k in ChanKind::all() {
                                    ui.selectable_value(&mut ch.kind, k, k.label());
                                }
                            });
                        ui.add(egui::TextEdit::singleline(&mut ch.label).desired_width(170.0));
                        ui.label(t("Ruhewert", "Rest value"));
                        ui.add(egui::DragValue::new(&mut ch.rest).range(0..=255));
                        let caps_btn = format!(
                            "{} {} ({})",
                            if fx.open_caps[c] { "⏷" } else { "⏵" },
                            t("Bereiche", "Ranges"),
                            ch.caps.len()
                        );
                        if ui.button(caps_btn).clicked() {
                            fx.open_caps[c] = !fx.open_caps[c];
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add_enabled(n > 1, egui::Button::new(t("Entfernen", "Remove"))).clicked() {
                                action = Some(RowAction::Remove(c));
                            }
                            if ui.add_enabled(c + 1 < n, egui::Button::new("⏷")).clicked() {
                                action = Some(RowAction::Down(c));
                            }
                            if ui.add_enabled(c > 0, egui::Button::new("⏶")).clicked() {
                                action = Some(RowAction::Up(c));
                            }
                        });
                    });
                    if fx.open_caps[c] {
                        ui.separator();
                        ui.indent("caps", |ui| caps_editor(ui, &mut fx.channels[c], bpm));
                    }
                });
            });
        }
        if ui.button(t("+ Kanal", "+ Channel")).clicked() {
            fx.channels.push(FixtureChannel {
                kind: ChanKind::Other,
                label: tf!("Kanal {}", "Channel {}", fx.channels.len() + 1),
                rest: 0,
                caps: Vec::new(),
            });
            fx.open_caps.push(false);
        }
    });

    match action {
        Some(RowAction::Up(i)) => {
            fx.channels.swap(i, i - 1);
            fx.open_caps.swap(i, i - 1);
        }
        Some(RowAction::Down(i)) => {
            fx.channels.swap(i, i + 1);
            fx.open_caps.swap(i, i + 1);
        }
        Some(RowAction::Remove(i)) => {
            fx.channels.remove(i);
            fx.open_caps.remove(i);
        }
        None => {}
    }

    if discard {
        if let Some((name, channels)) = fx.base.clone() {
            fx.name = name;
            fx.open_caps = vec![false; channels.len()];
            fx.channels = channels;
        }
    }
    if save {
        let r = config::write_fixture_txt(&show.dir, &fx.id, &fx.name, &fx.channels, None);
        if r.is_ok() {
            fx.base = Some((fx.name.clone(), fx.channels.clone()));
        }
        let id = fx.id.clone();
        report(st, shared, r.map(|_| tf!("fixtures/{id}.txt gespeichert", "fixtures/{id}.txt saved")));
    }
}

fn channel_overview(ui: &mut egui::Ui, channels: &[FixtureChannel]) {
    egui::Grid::new("chov").striped(true).show(ui, |ui| {
        for (i, ch) in channels.iter().enumerate() {
            ui.label(format!("{}", i + 1));
            ui.label(ch.label.as_str());
            ui.label(RichText::new(ch.kind.label()).weak());
            let caps: Vec<String> = ch
                .caps
                .iter()
                .take(4)
                .map(|c| format!("{}-{} {}", c.min, c.max, c.label))
                .collect();
            let more = if ch.caps.len() > 4 {
                format!(" … (+{})", ch.caps.len() - 4)
            } else {
                String::new()
            };
            ui.label(RichText::new(format!("{}{more}", caps.join(" · "))).small());
            ui.end_row();
        }
    });
}

fn effect_names() -> [&'static str; 7] {
    [
        t("Aus", "Off"),
        t("Offen", "Open"),
        t("Zu", "Closed"),
        "Strobe",
        t("Zufalls-Strobe", "Random strobe"),
        t("Farbe", "Colour"),
        "Text",
    ]
}

fn effect_index(e: &Effect) -> usize {
    match e {
        Effect::Nothing => 0,
        Effect::Open => 1,
        Effect::Closed => 2,
        Effect::Strobe { random: false, .. } => 3,
        Effect::Strobe { random: true, .. } => 4,
        Effect::Color(..) => 5,
        Effect::Text => 6,
    }
}

fn effect_from_index(i: usize, old: &Effect) -> Effect {
    let (from, to) = match old {
        Effect::Strobe { from, to, .. } => (*from, *to),
        _ => config::DEFAULT_STROBE,
    };
    match i {
        0 => Effect::Nothing,
        1 => Effect::Open,
        2 => Effect::Closed,
        3 => Effect::Strobe { from, to, random: false },
        4 => Effect::Strobe { from, to, random: true },
        5 => Effect::Color([255, 0, 0], None),
        _ => Effect::Text,
    }
}

fn caps_editor(ui: &mut egui::Ui, ch: &mut FixtureChannel, bpm: bool) {
    let names = effect_names();
    let mut remove: Option<usize> = None;
    egui::Grid::new("capsgrid").striped(true).show(ui, |ui| {
        for (k, cap) in ch.caps.iter_mut().enumerate() {
            ui.push_id(("cap", k), |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut cap.min).range(0..=255));
                    ui.label("–");
                    ui.add(egui::DragValue::new(&mut cap.max).range(0..=255));
                });
            });
            let mut idx = effect_index(&cap.effect);
            egui::ComboBox::from_id_salt(("eff", k))
                .selected_text(names[idx])
                .show_ui(ui, |ui| {
                    for (i, name) in names.iter().enumerate() {
                        ui.selectable_value(&mut idx, i, *name);
                    }
                });
            if idx != effect_index(&cap.effect) {
                cap.effect = effect_from_index(idx, &cap.effect);
            }
            ui.push_id(("par", k), |ui| {
                ui.horizontal(|ui| match &mut cap.effect {
                    Effect::Strobe { from, to, .. } => {
                        freq_drag(ui, from, bpm);
                        ui.label("→");
                        freq_drag(ui, to, bpm);
                    }
                    Effect::Color(a, b) => {
                        ui.color_edit_button_srgb(a);
                        let mut two = b.is_some();
                        if ui.checkbox(&mut two, t("2. Farbe", "2nd colour")).changed() {
                            *b = if two { Some([0, 0, 255]) } else { None };
                        }
                        if let Some(bb) = b {
                            ui.color_edit_button_srgb(bb);
                        }
                    }
                    _ => {}
                });
            });
            ui.add(
                egui::TextEdit::singleline(&mut cap.label)
                    .hint_text(t("Bezeichnung", "Label"))
                    .desired_width(170.0),
            );
            if ui.button(t("Entfernen", "Remove")).clicked() {
                remove = Some(k);
            }
            ui.end_row();
        }
    });
    if let Some(k) = remove {
        ch.caps.remove(k);
    }
    if ui.button(t("+ Bereich", "+ Range")).clicked() {
        let start = ch.caps.iter().map(|c| c.max as u16 + 1).max().unwrap_or(0).min(255) as u8;
        ch.caps.push(Capability {
            min: start,
            max: 255,
            effect: Effect::Text,
            label: String::new(),
        });
    }
    if ch.caps.iter().any(|c| c.min > c.max) {
        ui.colored_label(RED, t("„von“ ist größer als „bis“", "“from” is greater than “to”"));
    }
}

// ---------------------------------------------------------------------------
// QLC+ library
// ---------------------------------------------------------------------------

fn library_tab(ui: &mut egui::Ui, show: &Show, shared: &Shared, st: &mut UiState) {
    let lib = match library::get() {
        Ok(l) => l,
        Err(e) => {
            ui.colored_label(
                RED,
                tf!("Bibliothek konnte nicht geladen werden: {e}", "Library could not be loaded: {e}"),
            );
            return;
        }
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(t("Suche:", "Search:"));
        ui.add(
            egui::TextEdit::singleline(&mut st.lib_query)
                .hint_text(t(
                    "Hersteller, Modell oder Typ – z. B. „eurolite par“",
                    "manufacturer, model or type – e.g. “eurolite par”",
                ))
                .desired_width(320.0),
        );
        if !st.lib_query.is_empty() && ui.button(t("Leeren", "Clear")).clicked() {
            st.lib_query.clear();
        }
    });
    let query = st.lib_query.trim().to_string();
    if st.lib_last_query.as_deref() != Some(query.as_str()) {
        st.lib_hits = if query.is_empty() {
            Vec::new()
        } else {
            lib.search_grouped(&query)
        };
        st.lib_last_query = Some(query.clone());
    }
    let searching = !query.is_empty();
    let total: usize = if searching {
        st.lib_hits.iter().map(|(_, h)| h.len()).sum()
    } else {
        lib.entries.len()
    };
    ui.label(
        RichText::new(if searching {
            tf!(
                "{total} Treffer bei {} Herstellern",
                "{total} results from {} manufacturers",
                st.lib_hits.len()
            )
        } else {
            tf!(
                "{} Hersteller · {} Fixtures",
                "{} manufacturers · {} fixtures",
                lib.groups.len(),
                lib.entries.len()
            )
        })
        .weak(),
    );
    ui.separator();

    let height = ui.available_height();
    let mut clicked: Option<usize> = None;
    ui.columns(2, |cols| {
        // ---- left: manufacturer folders ----
        egui::ScrollArea::vertical()
            .id_salt("lib-tree")
            .max_height(height)
            .auto_shrink([false, false])
            .show(&mut cols[0], |ui| {
                let force_open = searching && total <= 150;
                let groups: Vec<(usize, &[usize])> = if searching {
                    st.lib_hits.iter().map(|(g, h)| (*g, h.as_slice())).collect()
                } else {
                    lib.groups.iter().enumerate().map(|(g, (_, h))| (g, h.as_slice())).collect()
                };
                if searching && groups.is_empty() {
                    ui.label(t("Nichts gefunden.", "Nothing found."));
                }
                for (gi, hits) in groups {
                    let name = &lib.groups[gi].0;
                    ui.push_id(("manu", gi), |ui| {
                        egui::CollapsingHeader::new(format!("{name}  ({})", hits.len()))
                            .open(if force_open { Some(true) } else { None })
                            .show(ui, |ui| {
                                for &i in hits {
                                    let e = &lib.entries[i];
                                    let label = if e.kind.is_empty() {
                                        e.model.clone()
                                    } else {
                                        format!("{}   ({})", e.model, e.kind)
                                    };
                                    if ui.selectable_label(st.lib_sel == Some(i), label).clicked() {
                                        clicked = Some(i);
                                    }
                                }
                            });
                    });
                }
            });

        // ---- right: details + patching ----
        egui::ScrollArea::vertical()
            .id_salt("lib-detail")
            .max_height(height)
            .auto_shrink([false, false])
            .show(&mut cols[1], |ui| library_detail(ui, lib, show, shared, st));
    });
    if let Some(i) = clicked {
        st.lib_sel = Some(i);
        st.info = None;
    }
}

fn library_detail(
    ui: &mut egui::Ui,
    lib: &'static library::Library,
    show: &Show,
    shared: &Shared,
    st: &mut UiState,
) {
    let Some(i) = st.lib_sel else {
        ui.label(t(
            "Links einen Hersteller aufklappen und ein Fixture wählen.",
            "Expand a manufacturer on the left and pick a fixture.",
        ));
        attribution(ui);
        return;
    };
    let e = &lib.entries[i];
    if st.lib_parsed.as_ref().map(|(j, _)| *j) != Some(i) {
        let parsed = qxf::parse_qxf(&library::slug(&e.manufacturer, &e.model), &lib.text(e)).map(|(ft, _)| ft);
        st.lib_parsed = Some((i, parsed));
        st.lib_mode = 0;
        st.lib_name = e.model.clone();
        st.lib_count = 1;
        st.lib_address = 0;
    }
    let ft = match &st.lib_parsed {
        Some((_, Ok(ft))) => ft.clone(),
        Some((_, Err(err))) => {
            ui.colored_label(RED, tf!("Fixture nicht lesbar: {err}", "Fixture not readable: {err}"));
            return;
        }
        None => return,
    };

    ui.label(RichText::new(format!("{} {}", e.manufacturer.replace('_', " "), e.model)).heading());
    ui.label(RichText::new(tf!("{} · {} Modi", "{} · {} modes", e.kind, ft.modes.len())).weak());
    ui.label(
        RichText::new(if e.custom {
            t("Quelle: eigene Definition (MIDI ArtNet Bridge)", "Source: own definition (MIDI ArtNet Bridge)")
        } else {
            t("Quelle: QLC+ Fixture-Bibliothek", "Source: QLC+ fixture library")
        })
        .small()
        .weak(),
    );
    ui.add_space(4.0);
    ui.label(t("Modus:", "Mode:"));
    for (mi, m) in ft.modes.iter().enumerate() {
        if ui
            .selectable_label(
                st.lib_mode == mi,
                tf!("{} ({} Kanäle)", "{} ({} channels)", m.name, m.channels.len()),
            )
            .clicked()
        {
            st.lib_mode = mi;
            st.lib_address = 0;
        }
    }
    let mode = &ft.modes[st.lib_mode.min(ft.modes.len() - 1)];
    let len = mode.channels.len() * st.lib_count.max(1);
    if st.lib_address == 0 {
        st.lib_address = show.next_free_address(&st.patch, st.lib_universe, len).unwrap_or(1);
    }

    egui::CollapsingHeader::new(tf!("Kanäle im Modus „{}“", "Channels in mode “{}”", mode.name))
        .default_open(false)
        .show(ui, |ui| channel_overview(ui, &mode.channels));

    ui.add_space(4.0);
    egui::Grid::new("libpatch").num_columns(2).show(ui, |ui| {
        ui.label(t("Name im Patch", "Name in patch"));
        ui.add(egui::TextEdit::singleline(&mut st.lib_name).desired_width(220.0));
        ui.end_row();
        ui.label(t("Universum", "Universe"));
        if ui
            .add(egui::DragValue::new(&mut st.lib_universe).range(0..=config::MAX_UNIVERSE))
            .changed()
        {
            st.lib_address = 0;
        }
        ui.end_row();
        ui.label(t("Startadresse", "Start address"));
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut st.lib_address).range(1..=512));
            if ui.button(t("nächste freie", "next free")).clicked() {
                st.lib_address = show.next_free_address(&st.patch, st.lib_universe, len).unwrap_or(1);
            }
        });
        ui.end_row();
        ui.label(t("Anzahl", "Count"));
        ui.add(egui::DragValue::new(&mut st.lib_count).range(1..=512));
        ui.end_row();
    });

    let install = |st: &mut UiState| -> Result<String, String> {
        config::install_qxf(&show.dir, &library::slug(&e.manufacturer, &e.model), &lib.text(e)).map_err(
            |err| {
                st.info = Some((false, err.clone()));
                err
            },
        )
    };

    ui.add_space(4.0);
    if ui.button(t("Hinzufügen und patchen", "Add and patch")).clicked() {
        if let Ok(id) = install(st) {
            st.patch.push(PatchEntry {
                name: if st.lib_name.trim().is_empty() {
                    e.model.clone()
                } else {
                    st.lib_name.trim().to_string()
                },
                fixture: id.clone(),
                universe: st.lib_universe,
                address: st.lib_address.max(1),
                mode: (ft.modes.len() > 1).then(|| mode.name.clone()),
                count: st.lib_count.max(1),
                spacing: None,
            });
            let r = config::write_patch(&show.dir, &st.patch);
            if r.is_ok() {
                st.patch_base = st.patch.clone();
            }
            let addr = st.lib_address;
            st.lib_address = 0;
            report(
                st,
                shared,
                r.map(|_| {
                    tf!(
                        "„{}“ ab Adresse {addr} gepatcht (fixtures/{id}.qxf)",
                        "“{}” patched from address {addr} (fixtures/{id}.qxf)",
                        e.model
                    )
                }),
            );
        }
    }
    if ui
        .button(t("Nur zu den Fixtures hinzufügen", "Only add to fixtures"))
        .clicked()
    {
        if let Ok(id) = install(st) {
            report(st, shared, Ok(tf!("fixtures/{id}.qxf angelegt", "fixtures/{id}.qxf created")));
        }
    }
    info_line(ui, st);
    attribution(ui);
}

fn attribution(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(tf!(
            "Fixture-Bibliothek: größtenteils aus QLC+ (qlcplus.org, Apache License 2.0), ergänzt um eigene Einträge · {}",
            "Fixture library: mostly from QLC+ (qlcplus.org, Apache License 2.0), plus own entries · {}",
            library::VERSION.lines().next().unwrap_or("")
        ))
        .weak()
        .small(),
    );
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

fn st_ip(text: &str) -> Option<std::net::Ipv4Addr> {
    text.trim().parse().ok()
}

fn settings_tab(ui: &mut egui::Ui, show: &Show, params: &BridgeParams, shared: &Arc<Shared>, st: &mut UiState) {
    let mut ip_ok = true;

    // ---- row 1: Art-Net (left) | nodes on the network (right) ----
    if let Some(s) = st.settings.as_mut() {
        let ip_text = &mut st.ip_text;
        ui.columns(2, |cols| {
            section(&mut cols[0], "Art-Net", |ui| {
                egui::Grid::new("netgrid").num_columns(2).show(ui, |ui| {
                    ui.label(t("IP des Nodes", "Node IP"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(ip_text).desired_width(130.0));
                        if ui.button("Broadcast").clicked() {
                            *ip_text = "255.255.255.255".into();
                        }
                    });
                    ui.end_row();
                    match st_ip(ip_text) {
                        Some(ip) => s.artnet_ip = ip,
                        None => {
                            ip_ok = false;
                            ui.label("");
                            ui.colored_label(RED, t("ungültige IP", "invalid IP"));
                            ui.end_row();
                        }
                    }
                    ui.label("Port");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut s.artnet_port).range(1..=65535));
                        if ui
                            .add_enabled(s.artnet_port != 6454, egui::Button::new(t("Standard", "Default")))
                            .clicked()
                        {
                            s.artnet_port = 6454;
                        }
                    });
                    ui.end_row();
                    ui.label(t("Wiederholrate", "Refresh rate"));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut s.refresh_hz)
                                .range(1.0..=44.0)
                                .speed(0.5)
                                .suffix(" Hz"),
                        );
                        if ui
                            .add_enabled((s.refresh_hz - 40.0).abs() > 0.01, egui::Button::new(t("Standard", "Default")))
                            .clicked()
                        {
                            s.refresh_hz = 40.0;
                        }
                    });
                    ui.end_row();
                    ui.label("MIDI-CCs");
                    ui.checkbox(&mut s.cc_input, t("als Werte annehmen (Fades)", "accept as values (fades)"));
                    ui.end_row();
                });
                if ui.button(t("Alles auf Standard", "Reset all to default")).clicked() {
                    *ip_text = "255.255.255.255".into();
                    s.artnet_port = 6454;
                    s.refresh_hz = 40.0;
                    s.cc_input = true;
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(t(
                        "Die IP des Art-Net-Nodes, nicht die deines Rechners. Broadcast erreicht alle Geräte, \
                         Unicast an die Node-IP ist zuverlässiger.",
                        "The IP of the Art-Net node, not of your computer. Broadcast reaches every device; \
                         unicast to the node IP is more reliable.",
                    ))
                    .weak()
                    .small(),
                );
            });

            section(&mut cols[1], t("Nodes im Netz", "Nodes on the network"), |ui| {
                nodes_panel(ui, shared, st_ip(ip_text), ip_text);
            });
        });
    }

    ui.add_space(6.0);

    // ---- row 2: MIDI mapping, 2 × 8 channels ----
    if let Some(s) = st.settings.as_mut() {
        section(ui, t("MIDI-Kanal → Universum / Block", "MIDI channel → universe / block"), |ui| {
            if ui.button(t("Standard-Zuordnung", "Default mapping")).clicked() {
                s.midi = config::default_midi_map();
            }
            ui.add_space(4.0);
            ui.columns(2, |cols| {
                for (col, range) in [(0usize, 0..8usize), (1, 8..16)] {
                    let ui = &mut cols[col];
                    egui::Grid::new(("midimap", col)).striped(true).num_columns(5).show(ui, |ui| {
                        for h in ["MIDI", t("aktiv", "on"), t("Universum", "Universe"), "Block", "DMX"] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for ch in range {
                            ui.label(format!("{}", ch + 1));
                            let mut on = s.midi[ch].is_some();
                            if ui.checkbox(&mut on, "").changed() {
                                s.midi[ch] = on.then_some(MidiTarget {
                                    universe: (ch / 4) as u16,
                                    block: (ch % 4) as u8,
                                });
                            }
                            match s.midi[ch].as_mut() {
                                Some(m) => {
                                    ui.add(egui::DragValue::new(&mut m.universe).range(0..=config::MAX_UNIVERSE));
                                    egui::ComboBox::from_id_salt(("blk", ch))
                                        .selected_text(format!("{}", m.block + 1))
                                        .width(40.0)
                                        .show_ui(ui, |ui| {
                                            for b in 0..4u8 {
                                                ui.selectable_value(&mut m.block, b, format!("{}", b + 1));
                                            }
                                        });
                                    ui.label(format!("{}–{}", m.first_slot() + 1, m.first_slot() + BLOCK_SIZE));
                                }
                                None => {
                                    ui.label("");
                                    ui.label("");
                                    ui.label(RichText::new(t("aus", "off")).weak());
                                }
                            }
                            ui.end_row();
                        }
                    });
                }
            });
        });
    }

    // ---- save / discard settings ----
    let dirty = st.settings != st.settings_base;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(dirty && ip_ok, egui::Button::new(t("Einstellungen speichern", "Save settings")))
            .clicked()
        {
            if let Some(s) = st.settings.clone() {
                let r = config::write_config(&show.dir, &s)
                    .map(|_| t("config.txt gespeichert", "config.txt saved").to_string());
                if r.is_ok() {
                    st.settings_base = Some(s);
                }
                report(st, shared, r);
            }
        }
        if ui.add_enabled(dirty, egui::Button::new(t("Verwerfen", "Discard"))).clicked() {
            let s = show.settings();
            st.ip_text = s.artnet_ip.to_string();
            st.settings = Some(s.clone());
            st.settings_base = Some(s);
        }
        if dirty {
            ui.colored_label(YELLOW, t("ungespeicherte Änderungen", "unsaved changes"));
        }
    });
    info_line(ui, st);
    ui.add_space(6.0);

    // ---- row 3: display (left) | config folder (right) ----
    let mut prefs = st.prefs;
    let mut window_reset = false;
    let mut dir_action: Option<u8> = None;
    let dir_edit = &mut st.dir_edit;
    ui.columns(2, |cols| {
        section(&mut cols[0], t("Darstellung", "Display"), |ui| {
            egui::Grid::new("dispgrid").num_columns(2).show(ui, |ui| {
                ui.label(t("Sprache", "Language"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut prefs.english, true, "English");
                    ui.radio_value(&mut prefs.english, false, "Deutsch");
                });
                ui.end_row();
                ui.label(t("Frequenzen", "Frequencies"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut prefs.bpm, false, "Hz");
                    ui.radio_value(&mut prefs.bpm, true, "BPM");
                });
                ui.end_row();
                ui.label(t("MIDI-Noten", "MIDI notes"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut prefs.notes, NoteFmt::Number, t("Zahl", "Number"));
                    ui.radio_value(&mut prefs.notes, NoteFmt::Name, "Name");
                    ui.radio_value(&mut prefs.notes, NoteFmt::Both, t("beides", "both"));
                });
                ui.end_row();
                ui.label(t("Fenster", "Window"));
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:.0} × {:.0}", prefs.window.0, prefs.window.1)).weak(),
                    );
                    if ui
                        .add_enabled(
                            prefs.window != config::DEFAULT_WINDOW,
                            egui::Button::new(t("Standardgröße", "Default size")),
                        )
                        .clicked()
                    {
                        prefs.window = config::DEFAULT_WINDOW;
                        window_reset = true;
                    }
                });
                ui.end_row();
                ui.label(t("Info unter Kacheln", "Info below tiles"));
                egui::ComboBox::from_id_salt("tileinfo2")
                    .selected_text(tile_info_label(prefs.tile))
                    .show_ui(ui, |ui| {
                        for ti in [TileInfo::Nothing, TileInfo::Address, TileInfo::Dimmer, TileInfo::Midi] {
                            ui.selectable_value(&mut prefs.tile, ti, tile_info_label(ti));
                        }
                    });
                ui.end_row();
            });
            ui.label(
                RichText::new(t(
                    "Zoom oben rechts, Fenstergröße per Griff unten rechts oder am Reaper-Fensterrand. \
                     Alles hier gilt für alle Projekte (ui.txt im Konfig-Ordner).",
                    "Zoom: top right. Window size: grip in the bottom-right corner or Reaper's window edge. \
                     Everything here applies to all projects (ui.txt in the config folder).",
                ))
                .weak()
                .small(),
            );
        });

        section(&mut cols[1], t("Konfig-Ordner", "Config folder"), |ui| {
            ui.add(egui::TextEdit::singleline(dir_edit).desired_width(f32::INFINITY));
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Übernehmen", "Apply")).clicked() {
                    dir_action = Some(1);
                }
                if ui.button(t("Standard", "Default")).clicked() {
                    dir_action = Some(2);
                }
                if ui.button(t("Ordner öffnen", "Open folder")).clicked() {
                    dir_action = Some(3);
                }
                if ui.button(t("Neu laden", "Reload")).clicked() {
                    dir_action = Some(4);
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Alle auf Ruhewert", "All to rest value")).clicked() {
                    dir_action = Some(5);
                }
                if ui
                    .button(t("Reaper-Notennamen exportieren", "Export Reaper note names"))
                    .clicked()
                {
                    dir_action = Some(6);
                }
            });
        });
    });

    if prefs != st.prefs {
        let lang_changed = prefs.english != st.prefs.english;
        st.prefs = prefs;
        i18n::set_english(prefs.english);
        save_prefs(params, st);
        if lang_changed {
            shared.reload_request.store(true, Relaxed);
        }
        if window_reset {
            request_window(params, &st.prefs);
        }
    }
    match dir_action {
        Some(1) => {
            if let Ok(mut d) = params.config_dir.write() {
                *d = st.dir_edit.trim().to_string();
            }
            shared.reload_request.store(true, Relaxed);
        }
        Some(2) => {
            if let Ok(mut d) = params.config_dir.write() {
                d.clear();
            }
            st.dir_edit = config::default_config_dir().display().to_string();
            shared.reload_request.store(true, Relaxed);
        }
        Some(3) => open_path(&artnet::effective_dir(params)),
        Some(4) => shared.reload_request.store(true, Relaxed),
        Some(5) => shared.reset_all_to_rest(),
        Some(6) => {
            let r = config::export_note_names(show).map(|p| {
                tf!(
                    "Notennamen gespeichert in {}",
                    "Note names saved to {}",
                    p.display()
                )
            });
            report(st, shared, r);
        }
        _ => {}
    }
    info_line(ui, st);

    ui.add_space(6.0);
    section(ui, t("Über", "About"), |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(concat!("MIDI ArtNet Bridge v", env!("CARGO_PKG_VERSION"))).strong());
            ui.label(RichText::new("· GPL-3.0").weak());
        });
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(t("Projekt auf GitHub", "Project on GitHub"))
                .on_hover_text(REPO_URL)
                .clicked()
            {
                open_url(REPO_URL);
            }
            if ui
                .button(t("Fehler melden", "Report a bug"))
                .on_hover_text(t(
                    "Öffnet ein vorausgefülltes Formular auf GitHub (Version und System sind schon eingetragen).",
                    "Opens a pre-filled form on GitHub (version and system already filled in).",
                ))
                .clicked()
            {
                open_url(&bug_report_url());
            }
            if ui
                .button(t("♥ Unterstützen", "♥ Support / donate"))
                .on_hover_text(SPONSOR_URL)
                .clicked()
            {
                open_url(SPONSOR_URL);
            }
        });
        attribution(ui);
    });
}

const REPO_URL: &str = "https://github.com/Der-Tino/MIDI-ArtNet-Bridge-Plugin";
const SPONSOR_URL: &str = "https://github.com/sponsors/Der-Tino";

/// Open a web address in the default browser
fn open_url(url: &str) {
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// GitHub "new issue" link with version and system already filled in
fn bug_report_url() -> String {
    let body = format!(
        "**What happened?**\n\n\n**What did you expect?**\n\n\n**Steps to reproduce**\n1. \n\n\
         ---\nPlugin: MIDI ArtNet Bridge v{}\nSystem: {} {}\nDAW + version: \nPlugin format (CLAP/VST3): \n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    format!(
        "{REPO_URL}/issues/new?labels=bug&title={}&body={}",
        url_encode("[Bug] "),
        url_encode(&body)
    )
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Own IP, subnet check and ArtPoll node search
fn nodes_panel(
    ui: &mut egui::Ui,
    shared: &Arc<Shared>,
    target: Option<std::net::Ipv4Addr>,
    ip_text: &mut String,
) {
    if let Some(tg) = target {
        match artnet::local_ip_for(tg) {
            Some(own) => {
                ui.label(RichText::new(tf!("Dein Rechner sendet von {own}", "Your computer sends from {own}")).small());
                let same_net = tg.is_broadcast() || own.octets()[..3] == tg.octets()[..3];
                if !same_net {
                    ui.colored_label(
                        YELLOW,
                        RichText::new(tf!(
                            "Node {tg} liegt in einem anderen Netz – er braucht eine IP wie {}.{}.{}.x",
                            "Node {tg} is in a different subnet – it needs an IP like {}.{}.{}.x",
                            own.octets()[0],
                            own.octets()[1],
                            own.octets()[2]
                        ))
                        .small(),
                    );
                }
            }
            None => {
                ui.colored_label(YELLOW, RichText::new(t("Kein Netzwerk gefunden.", "No network found.")).small());
            }
        }
    }

    ui.horizontal_wrapped(|ui| {
        let running = shared.discovery_running.load(Relaxed);
        if ui
            .add_enabled(
                !running,
                egui::Button::new(if running {
                    t("Suche läuft …", "Searching …")
                } else {
                    t("Nodes suchen", "Search nodes")
                }),
            )
            .clicked()
        {
            artnet::start_discovery(shared.clone());
        }
        if let Ok(s) = shared.discovery_status.lock() {
            if !running && !s.is_empty() {
                ui.label(RichText::new(s.as_str()).small());
            }
        }
    });

    let nodes = shared.nodes.lock().map(|n| n.clone()).unwrap_or_default();
    if !nodes.is_empty() {
        egui::Grid::new("nodes").striped(true).num_columns(3).show(ui, |ui| {
            for n in nodes {
                let name = if n.long_name.is_empty() { n.short_name.clone() } else { n.long_name.clone() };
                ui.label(RichText::new(n.ip.to_string()).monospace());
                ui.label(name);
                if ui.button(t("übernehmen", "use")).clicked() {
                    *ip_text = n.ip.to_string();
                }
                ui.end_row();
            }
        });
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new(t(
            "Tippen geht nicht? In Reaper im FX-Fenster „Send all keyboard input to plugin“ einschalten.",
            "Can't type? In Reaper's FX window enable “Send all keyboard input to plugin”.",
        ))
        .weak()
        .small(),
    );
}
