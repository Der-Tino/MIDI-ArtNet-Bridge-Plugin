//! GUI: Live-Vorschau, Patch-Editor, Fixture-Editor, QLC+ Bibliothek, Einstellungen.

use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText};
use nih_plug_egui::{create_egui_editor, widgets::ParamSlider};

use crate::artnet;
use crate::config::{
    self, Capability, ChanKind, Effect, FixtureChannel, FixtureType, Level, MidiTarget,
    PatchEntry, PatchedFixture, Settings, Show, UiPrefs, BLOCK_SIZE, UI_SCALES,
};
use crate::library;
use crate::preview;
use crate::qxf;
use crate::shared::{self, Shared, MAX_BUFS};
use crate::BridgeParams;

/// Layout-Fläche in Punkten – bleibt bei jeder Größe gleich, nur der Zoom ändert sich
pub const BASE_W: f32 = 880.0;
pub const BASE_H: f32 = 600.0;
/// egui ist von Haus aus klein – „100 %“ entspricht Zoom 1,25
pub const ZOOM_BASE: f32 = 1.25;

/// Zoom für eine Größenstufe (0.5 … 2.0)
pub fn zoom_for(scale: f32) -> f32 {
    scale * ZOOM_BASE
}

/// Fenstergröße (logische Pixel) für eine Größenstufe
pub fn window_size_for(scale: f32) -> (u32, u32) {
    let z = zoom_for(scale);
    ((BASE_W * z).round() as u32, (BASE_H * z).round() as u32)
}

const RED: Color32 = Color32::from_rgb(235, 90, 80);
const YELLOW: Color32 = Color32::from_rgb(235, 195, 70);
const GREEN: Color32 = Color32::from_rgb(100, 205, 120);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Live,
    Patch,
    Fixtures,
    Library,
    Settings,
}

/// Arbeitskopie eines Text-Fixtures im Editor
#[derive(Clone)]
struct FxEdit {
    id: String,
    name: String,
    channels: Vec<FixtureChannel>,
    open_caps: Vec<bool>,
    /// Stand beim Laden/Speichern – zum Erkennen ungespeicherter Änderungen
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
    bpm: bool,
    /// Strobe-Rechner: (Gerät, Kanal) → Wunschfrequenz in Hz
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
            // Ubuntu-Light (Standard) hat z. B. keine Pfeile → Hack als Ersatzschrift
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
                request_window(&params, st.prefs.scale);
                st.dir_edit = artnet::effective_dir(&params).display().to_string();
                st.rate_t = now;
                st.rate_pkts = shared.packets_sent.load(Relaxed);
                st.lib_count = 1;
                st.initialized = true;
            }

            // Paketrate statt Zähler
            if now - st.rate_t >= 1.0 {
                let p = shared.packets_sent.load(Relaxed);
                st.rate = (p.saturating_sub(st.rate_pkts)) as f32 / (now - st.rate_t) as f32;
                st.rate_pkts = p;
                st.rate_t = now;
            }

            st.bpm = st.prefs.bpm;
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
                        ui.label("Lade Konfiguration …");
                        return;
                    };
                    if st.tab == Tab::Library {
                        // eigene Scrollbereiche (Hersteller-Liste und Details)
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
        },
    )
}

// ---------------------------------------------------------------------------
// Allgemein
// ---------------------------------------------------------------------------

/// Neue Konfiguration geladen → Arbeitskopien ohne ungespeicherte
/// Änderungen aktualisieren.
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
        if let Some(t) = show.type_info(&fx.id).filter(|t| !t.is_qxf) {
            let loaded = (t.ft.name.clone(), t.ft.modes[0].channels.clone());
            if !fx.dirty() {
                fx.name = loaded.0.clone();
                fx.channels = loaded.1.clone();
                fx.open_caps.resize(fx.channels.len(), false);
            }
            fx.base = Some(loaded);
        }
    }
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

/// Velocity, die beim Note-On genau diesen DMX-Wert ergibt
fn velocity_for(dmx: u8) -> u8 {
    ((dmx as f32 * 127.0 / 255.0).round() as u8).clamp(1, 127)
}

/// Frequenz-Eingabe in der gewählten Einheit; intern immer Hz
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
        Err(e) => st.info = Some((false, format!("Fehler: {e}"))),
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

fn request_window(params: &BridgeParams, scale: f32) {
    nih_plug_egui::set_ui_zoom(zoom_for(scale));
    let want = window_size_for(scale);
    if params.editor_state.size() != want {
        params.editor_state.set_requested_size(want);
    }
}

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
        toggle(ui, setter, &params.output_enabled, "Art-Net Ausgabe");

        // Skalierung wie bei Waves: feste Stufen, oben rechts
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
            ui.label("Größe");
            if scale != st.prefs.scale {
                st.prefs.scale = scale;
                let _ = config::write_ui_prefs(&artnet::effective_dir(params), &st.prefs);
                request_window(params, scale);
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
        ui.label(format!("Art-Net an {target}"));
        ui.label("·");
        ui.colored_label(rate_col, format!("{:.0} Pakete/s", st.rate));
        ui.label("·");
        ui.label(format!("Universen [{}]", unis.join(", ")));
        if errors > 0 {
            ui.label("·");
            ui.colored_label(RED, format!("{errors} Sendefehler"));
        }
        if let Some(s) = show {
            let n = s.error_count();
            if n > 0 {
                ui.label("·");
                ui.colored_label(RED, format!("{n} Konfig-Fehler (siehe Live)"));
            }
        }
    });

    if !params.output_enabled.value() {
        ui.colored_label(YELLOW, "Ausgabe deaktiviert – es wird nichts gesendet.");
    } else if params.blackout.value() {
        ui.colored_label(RED, "BLACKOUT aktiv");
    }
    if let Some(e) = shared.net_error.lock().ok().and_then(|g| g.clone()) {
        ui.colored_label(RED, format!("Netzwerk: {e}"));
    }
    ui.label(RichText::new(last_midi_text(shared, show)).monospace().small());
}

fn last_midi_text(shared: &Shared, show: Option<&Show>) -> String {
    if shared.event_count.load(Relaxed) == 0 {
        return "Noch kein MIDI empfangen.".into();
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
        return format!("Letztes MIDI: {kind_s} · Kanal {}", ch + 1);
    }
    let dest = show
        .and_then(|s| {
            let t = s.midi[ch as usize]?;
            let slot = t.first_slot() + n as usize;
            Some(match s.lookup(t.universe, slot) {
                Some((f, c)) => format!("U{} DMX {} = {}: {}", t.universe, slot + 1, f.name, c.label),
                None => format!("U{} DMX {} (nicht gepatcht)", t.universe, slot + 1),
            })
        })
        .unwrap_or_else(|| "MIDI-Kanal nicht zugeordnet".into());
    format!(
        "Letztes MIDI: {kind_s} · Kanal {} · {} {n} ({}) → {v}  ·  {dest}",
        ch + 1,
        if kind == shared::EV_CC { "CC" } else { "Note" },
        config::note_name(n)
    )
}

fn tab_bar(ui: &mut egui::Ui, st: &mut UiState) {
    let patch_dirty = st.patch != st.patch_base;
    let settings_dirty = st.settings != st.settings_base;
    let fx_dirty = st.fx_edit.as_ref().is_some_and(|f| f.dirty());
    let tabs = [
        (Tab::Live, "Live", false),
        (Tab::Patch, "Patch", patch_dirty),
        (Tab::Fixtures, "Fixtures", fx_dirty),
        (Tab::Library, "QLC+ Bibliothek", false),
        (Tab::Settings, "Einstellungen", settings_dirty),
    ];
    ui.horizontal_wrapped(|ui| {
        for (t, label, dirty) in tabs {
            let text = if dirty { format!("{label} •") } else { label.to_string() };
            if ui
                .selectable_label(st.tab == t, RichText::new(text).strong())
                .clicked()
            {
                st.tab = t;
                st.info = None;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Live
// ---------------------------------------------------------------------------

fn live_tab(ui: &mut egui::Ui, show: &Show, shared: &Shared, master: f32, st: &mut UiState) {
    let bpm = st.bpm;
    for m in show.messages.iter().filter(|m| m.level != Level::Info) {
        match m.level {
            Level::Error => ui.colored_label(RED, format!("Fehler: {}", m.text)),
            _ => ui.colored_label(YELLOW, format!("Hinweis: {}", m.text)),
        };
    }

    if show.fixtures.is_empty() {
        ui.label("Keine Geräte gepatcht – im Tab „Patch“ oder „QLC+ Bibliothek“ hinzufügen.");
        return;
    }
    let t = ui.input(|i| i.time);
    let scale = 1.0;

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

    ui.heading("Bühne");
    ui.horizontal_wrapped(|ui| {
        for (f, vals) in show.fixtures.iter().zip(&all_vals) {
            let p = preview::compute(f, vals, t);
            ui.vertical(|ui| {
                ui.set_max_width(80.0 * scale);
                let (rect, resp) = ui.allocate_exact_size(
                    egui::vec2(72.0 * scale, 44.0 * scale),
                    egui::Sense::hover(),
                );
                let fill = if p.lit {
                    Color32::from_rgb(p.rgb[0], p.rgb[1], p.rgb[2])
                } else {
                    Color32::from_rgb(18, 18, 18)
                };
                ui.painter().rect_filled(rect, 4.0, fill);
                if let preview::Shutter::Blink { hz, .. } = p.shutter {
                    ui.painter().text(
                        rect.right_bottom() - egui::vec2(3.0, 2.0),
                        egui::Align2::RIGHT_BOTTOM,
                        freq_text(hz, bpm),
                        egui::FontId::proportional(11.0 * scale),
                        Color32::from_rgb(240, 240, 240),
                    );
                }
                resp.on_hover_text(format!(
                    "{}\n{} · {}\nU{} · DMX {}\n{}",
                    f.name,
                    f.type_name,
                    f.mode_name,
                    f.universe,
                    f.address,
                    midi_range_text(show, f)
                ));
                ui.label(RichText::new(f.name.as_str()).small());
            });
        }
    });

    ui.add_space(8.0);
    ui.heading("Fader");
    ui.label(
        RichText::new("Fader setzen den Wert direkt – das nächste MIDI-Event auf dem Kanal überschreibt ihn.")
            .weak()
            .small(),
    );

    for (i, (f, vals)) in show.fixtures.iter().zip(&all_vals).enumerate() {
        let buf = shared.buffer_for_universe(f.universe);
        let title = format!(
            "{}   ·   {} ({})   ·   U{} DMX {}–{}   ·   {}",
            f.name,
            f.type_name,
            f.mode_name,
            f.universe,
            f.address,
            f.address as usize + f.channels.len() - 1,
            midi_range_text(show, f)
        );
        ui.push_id(i, |ui| {
            egui::CollapsingHeader::new(title)
                .default_open(show.fixtures.len() <= 2)
                .show(ui, |ui| {
                    if buf.is_none() {
                        ui.colored_label(
                            YELLOW,
                            "Universum ist keinem MIDI-Kanal zugeordnet – wird nicht gesendet.",
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
                                    Some((mc, n)) => {
                                        format!("MIDI {} · Note {n} ({})", mc + 1, config::note_name(n))
                                    }
                                    None => "keine MIDI-Zuordnung".into(),
                                })
                                .weak(),
                            );
                            ui.end_row();

                            // Strobe-Rechner: Wunschfrequenz → DMX-Wert / Velocity
                            let strobe_caps: Vec<&Capability> = ch
                                .caps
                                .iter()
                                .filter(|k| matches!(k.effect, Effect::Strobe { .. }))
                                .collect();
                            if !strobe_caps.is_empty() {
                                let target = st.strobe_target.entry((i, c)).or_insert(8.0);
                                ui.label("");
                                ui.label(RichText::new("Wunschfrequenz").weak());
                                ui.horizontal(|ui| {
                                    freq_drag(ui, target, bpm);
                                    // Umrechnung in die jeweils andere Einheit
                                    ui.label(RichText::new(format!("= {}", freq_text(*target, !bpm))).weak());
                                });
                                let hit = strobe_caps.iter().find_map(|k| k.value_for_hz(*target));
                                match hit {
                                    Some(dmx) => {
                                        let vel = velocity_for(dmx);
                                        // Velocity hat nur 128 Stufen → tatsächliche Frequenz zeigen
                                        let dmx_from_vel = shared::to_dmx(vel as f32 / 127.0);
                                        let real = strobe_caps
                                            .iter()
                                            .find_map(|k| k.strobe_hz(dmx_from_vel).filter(|_| k.cap_contains(dmx_from_vel)))
                                            .map(|hz| format!(" (≈ {})", freq_text(hz, bpm)))
                                            .unwrap_or_default();
                                        ui.label(format!("→ DMX {dmx}"));
                                        ui.label(RichText::new(format!("Velocity {vel}{real}")).strong());
                                        if let Some(b) = buf {
                                            if ui.button("setzen").clicked() {
                                                shared.set_manual(b, slot, dmx);
                                            }
                                        } else {
                                            ui.label("");
                                        }
                                    }
                                    None => {
                                        let (lo, hi) = strobe_caps.iter().fold((f32::MAX, 0f32), |(lo, hi), k| match k.effect {
                                            Effect::Strobe { from, to, .. } => (lo.min(from.min(to)), hi.max(from.max(to))),
                                            _ => (lo, hi),
                                        });
                                        ui.colored_label(
                                            YELLOW,
                                            format!("außerhalb ({}–{})", freq_text(lo, bpm), freq_text(hi, bpm)),
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
}

fn midi_range_text(show: &Show, f: &PatchedFixture) -> String {
    let last = f.channels.len() - 1;
    match (
        show.midi_for(f.universe, f.slot(0)),
        show.midi_for(f.universe, f.slot(last)),
    ) {
        (Some((c1, n1)), Some((c2, n2))) if c1 == c2 => format!(
            "MIDI {} · Noten {n1}–{n2} ({}–{})",
            c1 + 1,
            config::note_name(n1),
            config::note_name(n2)
        ),
        (Some((c1, n1)), Some((c2, n2))) => format!(
            "MIDI {}/{n1} … MIDI {}/{n2} (über Blockgrenze)",
            c1 + 1,
            c2 + 1
        ),
        _ => "MIDI: teilweise/keine Zuordnung".into(),
    }
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
        if ui.button("+ Gerät").clicked() {
            if let Some(t) = show.types.first() {
                let len = t.ft.modes.first().map_or(1, |m| m.channels.len());
                let address = show.next_free_address(&st.patch, 0, len).unwrap_or(1);
                st.patch.push(PatchEntry {
                    name: t.ft.name.clone(),
                    fixture: t.ft.id.clone(),
                    universe: 0,
                    address,
                    mode: None,
                    count: 1,
                    spacing: None,
                });
            }
        }
        if ui.add_enabled(dirty, egui::Button::new("Speichern")).clicked() {
            let r = config::write_patch(&show.dir, &st.patch).map(|_| "patch.txt gespeichert".to_string());
            if r.is_ok() {
                st.patch_base = st.patch.clone();
            }
            report(st, shared, r);
        }
        if ui.add_enabled(dirty, egui::Button::new("Verwerfen")).clicked() {
            st.patch = show.patch.clone();
            st.patch_base = show.patch.clone();
        }
        if dirty {
            ui.colored_label(YELLOW, "ungespeicherte Änderungen");
        }
    });
    info_line(ui, st);
    ui.label(
        RichText::new("Neue Gerätetypen findest du im Tab „QLC+ Bibliothek“ oder legst sie unter „Fixtures“ an.")
            .weak()
            .small(),
    );
    ui.add_space(4.0);

    if st.patch.is_empty() {
        ui.label("Noch keine Geräte gepatcht.");
    }

    let mut action: Option<RowAction> = None;
    let n = st.patch.len();
    egui::Grid::new("patchgrid")
        .striped(true)
        .num_columns(9)
        .show(ui, |ui| {
            for h in ["Name", "Fixture", "Modus", "Univ.", "Adresse", "Anzahl", "Kanäle", "DMX", ""] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();

            for i in 0..n {
                let e = &mut st.patch[i];
                ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(150.0));

                let tinfo = show.type_info(&e.fixture);
                let fx_label = tinfo
                    .map(|t| t.ft.name.clone())
                    .unwrap_or_else(|| format!("‹fehlt: {}›", e.fixture));
                egui::ComboBox::from_id_salt(("fx", i))
                    .selected_text(fx_label)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for t in &show.types {
                            let src = if t.is_qxf { "QLC+" } else { "txt" };
                            if ui
                                .selectable_label(e.fixture == t.ft.id, format!("{}  ({src})", t.ft.name))
                                .clicked()
                            {
                                e.fixture = t.ft.id.clone();
                                e.mode = None;
                            }
                        }
                    });

                match tinfo.filter(|t| t.ft.modes.len() > 1) {
                    Some(t) => {
                        let cur = config::pick_mode(&t.ft, e.mode.as_deref())
                            .map(|m| m.name.clone())
                            .unwrap_or_else(|| "?".into());
                        egui::ComboBox::from_id_salt(("mode", i))
                            .selected_text(cur.clone())
                            .show_ui(ui, |ui| {
                                for m in &t.ft.modes {
                                    if ui
                                        .selectable_label(cur == m.name, format!("{} ({} Kan.)", m.name, m.channels.len()))
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
                    if ui.button("Entfernen").clicked() {
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

    let msgs: Vec<_> = show
        .messages
        .iter()
        .filter(|m| m.level != Level::Info && (m.text.starts_with("patch.txt") || m.text.contains("Überschneidung") || m.text.contains("MIDI-Zuordnung")))
        .collect();
    if !msgs.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new("Stand der gespeicherten Datei:").weak());
        for m in msgs {
            let col = if m.level == Level::Error { RED } else { YELLOW };
            ui.colored_label(col, m.text.as_str());
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn load_fx_edit(show: &Show, id: &str) -> Option<FxEdit> {
    let t = show.type_info(id).filter(|t| !t.is_qxf)?;
    let channels = t.ft.modes[0].channels.clone();
    Some(FxEdit {
        id: id.to_string(),
        name: t.ft.name.clone(),
        open_caps: vec![false; channels.len()],
        base: Some((t.ft.name.clone(), channels.clone())),
        channels,
    })
}

fn fixtures_tab(ui: &mut egui::Ui, show: &Show, shared: &Shared, st: &mut UiState) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Fixture:");
        let sel_label = st
            .fx_sel
            .as_ref()
            .and_then(|id| show.type_info(id).map(|t| t.ft.name.clone()).or_else(|| Some(format!("{id} (neu)"))))
            .unwrap_or_else(|| "– auswählen –".into());
        let mut picked: Option<String> = None;
        egui::ComboBox::from_id_salt("fxsel")
            .selected_text(sel_label)
            .width(300.0)
            .show_ui(ui, |ui| {
                for t in &show.types {
                    let src = if t.is_qxf { "QLC+" } else { "txt" };
                    if ui
                        .selectable_label(st.fx_sel.as_deref() == Some(t.ft.id.as_str()), format!("{}  ({src})", t.ft.name))
                        .clicked()
                    {
                        picked = Some(t.ft.id.clone());
                    }
                }
            });
        if let Some(id) = picked {
            st.fx_edit = load_fx_edit(show, &id);
            st.fx_sel = Some(id);
            st.qxf_copy_mode = 0;
            st.info = None;
        }

        ui.add_space(20.0);
        ui.add(
            egui::TextEdit::singleline(&mut st.new_fx_name)
                .hint_text("Name für neues Fixture")
                .desired_width(200.0),
        );
        if ui
            .add_enabled(!st.new_fx_name.trim().is_empty(), egui::Button::new("Neu anlegen"))
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
    });
    info_line(ui, st);
    ui.separator();

    // QLC+-Fixture: nur ansehen, als Textkopie bearbeitbar
    if st.fx_edit.is_none() {
        if let Some(t) = st.fx_sel.as_deref().and_then(|id| show.type_info(id)) {
            if t.is_qxf {
                ui.label(RichText::new(t.ft.name.as_str()).heading());
                ui.label(format!(
                    "QLC+-Definition (fixtures/{}). Zum Bearbeiten eine Textkopie eines Modus anlegen:",
                    t.file
                ));
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("qxfmode")
                        .selected_text(
                            t.ft.modes
                                .get(st.qxf_copy_mode)
                                .map(|m| m.name.clone())
                                .unwrap_or_default(),
                        )
                        .show_ui(ui, |ui| {
                            for (i, m) in t.ft.modes.iter().enumerate() {
                                ui.selectable_value(&mut st.qxf_copy_mode, i, format!("{} ({} Kan.)", m.name, m.channels.len()));
                            }
                        });
                    if ui.button("Als bearbeitbare Textdatei kopieren").clicked() {
                        if let Some(m) = t.ft.modes.get(st.qxf_copy_mode) {
                            let id = config::unique_fixture_id(&show.dir, &format!("{}_eigen", t.ft.id));
                            st.fx_edit = Some(FxEdit {
                                id: id.clone(),
                                name: format!("{} ({})", t.ft.name, m.name),
                                open_caps: vec![false; m.channels.len()],
                                channels: m.channels.clone(),
                                base: None,
                            });
                            st.fx_sel = Some(id);
                        }
                    }
                });
                if let Some(m) = t.ft.modes.get(st.qxf_copy_mode) {
                    channel_overview(ui, &m.channels);
                }
                return;
            }
        }
        ui.label("Wähle oben ein Fixture aus oder lege ein neues an.");
        return;
    }

    let bpm = st.bpm;
    let Some(fx) = st.fx_edit.as_mut() else { return };
    let dirty = fx.dirty();
    let mut save = false;
    let mut discard = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("Name:");
        ui.add(egui::TextEdit::singleline(&mut fx.name).desired_width(260.0));
        ui.label(RichText::new(format!("Datei: fixtures/{}.txt", fx.id)).weak());
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("+ Kanal").clicked() {
            fx.channels.push(FixtureChannel {
                kind: ChanKind::Other,
                label: format!("Kanal {}", fx.channels.len() + 1),
                rest: 0,
                caps: Vec::new(),
            });
            fx.open_caps.push(false);
        }
        save = ui.add_enabled(dirty, egui::Button::new("Speichern")).clicked();
        discard = ui.add_enabled(dirty && fx.base.is_some(), egui::Button::new("Verwerfen")).clicked();
        if dirty {
            ui.colored_label(YELLOW, "ungespeicherte Änderungen");
        }
    });

    fx.open_caps.resize(fx.channels.len(), false);
    let mut action: Option<RowAction> = None;
    let n = fx.channels.len();
    for c in 0..n {
        ui.push_id(("ch", c), |ui| {
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
                ui.label("Ruhewert");
                ui.add(egui::DragValue::new(&mut ch.rest).range(0..=255));
                let caps_btn = format!(
                    "{} Bereiche ({})",
                    if fx.open_caps[c] { "⏷" } else { "⏵" },
                    ch.caps.len()
                );
                if ui.button(caps_btn).clicked() {
                    fx.open_caps[c] = !fx.open_caps[c];
                }
                if ui.add_enabled(c > 0, egui::Button::new("⏶")).clicked() {
                    action = Some(RowAction::Up(c));
                }
                if ui.add_enabled(c + 1 < n, egui::Button::new("⏷")).clicked() {
                    action = Some(RowAction::Down(c));
                }
                if ui.add_enabled(n > 1, egui::Button::new("Entfernen")).clicked() {
                    action = Some(RowAction::Remove(c));
                }
            });
            if fx.open_caps[c] {
                ui.indent("caps", |ui| caps_editor(ui, &mut fx.channels[c], bpm));
            }
        });
    }
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
        if let Some((n, c)) = fx.base.clone() {
            fx.name = n;
            fx.open_caps = vec![false; c.len()];
            fx.channels = c;
        }
    }
    if save {
        let r = config::write_fixture_txt(&show.dir, &fx.id, &fx.name, &fx.channels, None);
        if r.is_ok() {
            fx.base = Some((fx.name.clone(), fx.channels.clone()));
        }
        let id = fx.id.clone();
        report(st, shared, r.map(|_| format!("fixtures/{id}.txt gespeichert")));
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

const EFFECTS: [&str; 7] = ["Aus", "Offen", "Zu", "Strobe", "Zufalls-Strobe", "Farbe", "Text"];

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
    let mut remove: Option<usize> = None;
    for (k, cap) in ch.caps.iter_mut().enumerate() {
        ui.push_id(("cap", k), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::DragValue::new(&mut cap.min).range(0..=255));
                ui.label("–");
                ui.add(egui::DragValue::new(&mut cap.max).range(0..=255));
                let mut idx = effect_index(&cap.effect);
                egui::ComboBox::from_id_salt("eff")
                    .selected_text(EFFECTS[idx])
                    .show_ui(ui, |ui| {
                        for (i, name) in EFFECTS.iter().enumerate() {
                            ui.selectable_value(&mut idx, i, *name);
                        }
                    });
                if idx != effect_index(&cap.effect) {
                    cap.effect = effect_from_index(idx, &cap.effect);
                }
                match &mut cap.effect {
                    Effect::Strobe { from, to, .. } => {
                        freq_drag(ui, from, bpm);
                        ui.label("→");
                        freq_drag(ui, to, bpm);
                    }
                    Effect::Color(a, b) => {
                        ui.color_edit_button_srgb(a);
                        let mut two = b.is_some();
                        if ui.checkbox(&mut two, "2. Farbe").changed() {
                            *b = if two { Some([0, 0, 255]) } else { None };
                        }
                        if let Some(bb) = b {
                            ui.color_edit_button_srgb(bb);
                        }
                    }
                    _ => {}
                }
                ui.add(egui::TextEdit::singleline(&mut cap.label).hint_text("Bezeichnung").desired_width(170.0));
                if ui.button("Entfernen").clicked() {
                    remove = Some(k);
                }
            });
        });
    }
    if let Some(k) = remove {
        ch.caps.remove(k);
    }
    if ui.button("+ Bereich").clicked() {
        let start = ch.caps.iter().map(|c| c.max as u16 + 1).max().unwrap_or(0).min(255) as u8;
        ch.caps.push(Capability {
            min: start,
            max: 255,
            effect: Effect::Text,
            label: String::new(),
        });
    }
    if ch.caps.iter().any(|c| c.min > c.max) {
        ui.colored_label(RED, "„von“ ist größer als „bis“");
    }
}

// ---------------------------------------------------------------------------
// QLC+ Bibliothek
// ---------------------------------------------------------------------------

fn library_tab(ui: &mut egui::Ui, show: &Show, shared: &Shared, st: &mut UiState) {
    let lib = match library::get() {
        Ok(l) => l,
        Err(e) => {
            ui.colored_label(RED, format!("Bibliothek konnte nicht geladen werden: {e}"));
            return;
        }
    };

    ui.horizontal_wrapped(|ui| {
        ui.label("Suche:");
        ui.add(
            egui::TextEdit::singleline(&mut st.lib_query)
                .hint_text("Hersteller, Modell oder Typ – z. B. „eurolite par“")
                .desired_width(320.0),
        );
        if !st.lib_query.is_empty() && ui.button("Leeren").clicked() {
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
            format!("{total} Treffer bei {} Herstellern", st.lib_hits.len())
        } else {
            format!("{} Hersteller · {} Fixtures", lib.groups.len(), lib.entries.len())
        })
        .weak(),
    );
    ui.separator();

    let height = ui.available_height();
    let mut clicked: Option<usize> = None;
    ui.columns(2, |cols| {
        // ---- links: Hersteller-Ordner ----
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
                    ui.label("Nichts gefunden.");
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

        // ---- rechts: Details + Patchen ----
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
        ui.label("Links einen Hersteller aufklappen und ein Fixture wählen.");
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
            ui.colored_label(RED, format!("Fixture nicht lesbar: {err}"));
            return;
        }
        None => return,
    };

    ui.label(RichText::new(format!("{} {}", e.manufacturer.replace('_', " "), e.model)).heading());
    ui.label(RichText::new(format!("{} · {} Modi", e.kind, ft.modes.len())).weak());
    ui.add_space(4.0);
    ui.label("Modus:");
    for (mi, m) in ft.modes.iter().enumerate() {
        if ui
            .selectable_label(st.lib_mode == mi, format!("{} ({} Kanäle)", m.name, m.channels.len()))
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

    egui::CollapsingHeader::new(format!("Kanäle im Modus „{}“", mode.name))
        .default_open(false)
        .show(ui, |ui| channel_overview(ui, &mode.channels));

    ui.add_space(4.0);
    egui::Grid::new("libpatch").num_columns(2).show(ui, |ui| {
        ui.label("Name im Patch");
        ui.add(egui::TextEdit::singleline(&mut st.lib_name).desired_width(220.0));
        ui.end_row();
        ui.label("Universum");
        if ui.add(egui::DragValue::new(&mut st.lib_universe).range(0..=config::MAX_UNIVERSE)).changed() {
            st.lib_address = 0;
        }
        ui.end_row();
        ui.label("Startadresse");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut st.lib_address).range(1..=512));
            if ui.button("nächste freie").clicked() {
                st.lib_address = show.next_free_address(&st.patch, st.lib_universe, len).unwrap_or(1);
            }
        });
        ui.end_row();
        ui.label("Anzahl");
        ui.add(egui::DragValue::new(&mut st.lib_count).range(1..=512));
        ui.end_row();
    });

    let install = |st: &mut UiState| -> Result<String, String> {
        config::install_qxf(&show.dir, &library::slug(&e.manufacturer, &e.model), &lib.text(e))
            .map_err(|err| {
                st.info = Some((false, err.clone()));
                err
            })
    };

    ui.add_space(4.0);
    if ui.button("Hinzufügen und patchen").clicked() {
        if let Ok(id) = install(st) {
            st.patch.push(PatchEntry {
                name: if st.lib_name.trim().is_empty() { e.model.clone() } else { st.lib_name.trim().to_string() },
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
                r.map(|_| format!("„{}“ ab Adresse {addr} gepatcht (fixtures/{id}.qxf)", e.model)),
            );
        }
    }
    if ui.button("Nur zu den Fixtures hinzufügen").clicked() {
        if let Ok(id) = install(st) {
            report(st, shared, Ok(format!("fixtures/{id}.qxf angelegt")));
        }
    }
    info_line(ui, st);
    attribution(ui);
}

fn attribution(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(format!(
            "Fixture-Daten: QLC+ (qlcplus.org), Apache License 2.0 · {}",
            library::VERSION.lines().next().unwrap_or("")
        ))
        .weak()
        .small(),
    );
}

// ---------------------------------------------------------------------------
// Einstellungen
// ---------------------------------------------------------------------------

fn st_ip(text: &str) -> Option<std::net::Ipv4Addr> {
    text.trim().parse().ok()
}

/// Erklärt, welche IP wohin gehört, und bietet die Node-Suche an.
fn network_help(ui: &mut egui::Ui, shared: &Arc<Shared>, target: Option<std::net::Ipv4Addr>, ip_text: &mut String) {
    ui.label(
        RichText::new(
            "Hier gehört die IP des Art-Net-Nodes hin (z. B. deines Pknight), nicht die deines Rechners. \
             255.255.255.255 = Broadcast: geht an alle Geräte im Netz – funktioniert meistens, \
             Unicast an die Node-IP ist aber zuverlässiger.",
        )
        .small(),
    );
    if let Some(t) = target {
        match artnet::local_ip_for(t) {
            Some(own) => {
                let same_net = t.is_broadcast() || own.octets()[..3] == t.octets()[..3];
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Dein Rechner sendet von {own}")).small());
                    if !same_net {
                        ui.colored_label(
                            YELLOW,
                            RichText::new(format!(
                                "– Node {t} liegt in einem anderen Netz. Der Node braucht eine IP wie {}.{}.{}.x",
                                own.octets()[0],
                                own.octets()[1],
                                own.octets()[2]
                            ))
                            .small(),
                        );
                    }
                });
            }
            None => {
                ui.colored_label(YELLOW, RichText::new("Kein Netzwerk gefunden.").small());
            }
        }
    }

    ui.horizontal_wrapped(|ui| {
        let running = shared.discovery_running.load(Relaxed);
        if ui
            .add_enabled(!running, egui::Button::new(if running { "Suche läuft …" } else { "Nodes im Netz suchen" }))
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
    for n in nodes {
        ui.horizontal_wrapped(|ui| {
            let name = if n.long_name.is_empty() { n.short_name.clone() } else { n.long_name.clone() };
            ui.label(RichText::new(format!("{}  ·  {name}", n.ip)).monospace());
            if ui.button("übernehmen").clicked() {
                *ip_text = n.ip.to_string();
            }
        });
    }
    ui.label(
        RichText::new(
            "Tippen in Eingabefelder geht nicht? Reaper gibt die Tastatur sonst nicht ans Plugin weiter: \
             im FX-Fenster die Option „Send all keyboard input to plugin“ einschalten.",
        )
        .weak()
        .small(),
    );
}

fn settings_tab(ui: &mut egui::Ui, show: &Show, params: &BridgeParams, shared: &Arc<Shared>, st: &mut UiState) {
    ui.heading("Art-Net");
    let mut ip_ok = true;
    if let Some(s) = st.settings.as_mut() {
        egui::Grid::new("netgrid").num_columns(2).show(ui, |ui| {
            ui.label("IP-Adresse des Nodes");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.ip_text).desired_width(160.0));
                match st.ip_text.trim().parse() {
                    Ok(ip) => s.artnet_ip = ip,
                    Err(_) => {
                        ip_ok = false;
                        ui.colored_label(RED, "ungültige IP");
                    }
                }
                if ui.button("Broadcast").clicked() {
                    st.ip_text = "255.255.255.255".into();
                }
            });
            ui.end_row();
            ui.label("Port");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.artnet_port).range(1..=65535));
                if ui.add_enabled(s.artnet_port != 6454, egui::Button::new("Standard (6454)")).clicked() {
                    s.artnet_port = 6454;
                }
            });
            ui.end_row();
            ui.label("Wiederholrate");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.refresh_hz).range(1.0..=44.0).speed(0.5).suffix(" Hz"));
                if ui.add_enabled((s.refresh_hz - 40.0).abs() > 0.01, egui::Button::new("Standard (40 Hz)")).clicked() {
                    s.refresh_hz = 40.0;
                }
            });
            ui.end_row();
            ui.label("MIDI-CCs als Werte");
            ui.checkbox(&mut s.cc_input, "annehmen (für Fades per CC-Lane)");
            ui.end_row();
        });
        if ui.button("Alle Netzwerk-Einstellungen auf Standard").clicked() {
            st.ip_text = "255.255.255.255".into();
            s.artnet_port = 6454;
            s.refresh_hz = 40.0;
            s.cc_input = true;
        }
        network_help(ui, shared, st_ip(&st.ip_text), &mut st.ip_text);

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.heading("MIDI-Kanal → Universum / Block");
            if ui.button("Standard-Zuordnung").clicked() {
                s.midi = config::default_midi_map();
            }
        });
        egui::Grid::new("midimap").striped(true).num_columns(5).show(ui, |ui| {
            for h in ["MIDI", "aktiv", "Universum", "Block", "DMX"] {
                ui.label(RichText::new(h).strong());
            }
            ui.end_row();
            for ch in 0..16 {
                ui.label(format!("Kanal {}", ch + 1));
                let mut on = s.midi[ch].is_some();
                if ui.checkbox(&mut on, "").changed() {
                    s.midi[ch] = on.then_some(MidiTarget {
                        universe: (ch / 4) as u16,
                        block: (ch % 4) as u8,
                    });
                }
                match s.midi[ch].as_mut() {
                    Some(t) => {
                        ui.add(egui::DragValue::new(&mut t.universe).range(0..=config::MAX_UNIVERSE));
                        egui::ComboBox::from_id_salt(("blk", ch))
                            .selected_text(format!("{}", t.block + 1))
                            .width(50.0)
                            .show_ui(ui, |ui| {
                                for b in 0..4u8 {
                                    ui.selectable_value(&mut t.block, b, format!("{}", b + 1));
                                }
                            });
                        ui.label(format!(
                            "{}–{}",
                            t.first_slot() + 1,
                            t.first_slot() + BLOCK_SIZE
                        ));
                    }
                    None => {
                        ui.label("");
                        ui.label("");
                        ui.label(RichText::new("aus").weak());
                    }
                }
                ui.end_row();
            }
        });
    }

    let dirty = st.settings != st.settings_base;
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(dirty && ip_ok, egui::Button::new("Speichern")).clicked() {
            if let Some(s) = st.settings.clone() {
                let r = config::write_config(&show.dir, &s).map(|_| "config.txt gespeichert".to_string());
                if r.is_ok() {
                    st.settings_base = Some(s);
                }
                report(st, shared, r);
            }
        }
        if ui.add_enabled(dirty, egui::Button::new("Verwerfen")).clicked() {
            let s = show.settings();
            st.ip_text = s.artnet_ip.to_string();
            st.settings = Some(s.clone());
            st.settings_base = Some(s);
        }
        if dirty {
            ui.colored_label(YELLOW, "ungespeicherte Änderungen");
        }
    });
    info_line(ui, st);

    ui.add_space(10.0);
    ui.heading("Darstellung");
    ui.horizontal_wrapped(|ui| {
        ui.label("Frequenzen als");
        let mut bpm = st.prefs.bpm;
        let a = ui.radio_value(&mut bpm, false, "Hz").changed();
        let b = ui.radio_value(&mut bpm, true, "BPM").changed();
        if a || b {
            st.prefs.bpm = bpm;
            st.bpm = bpm;
            let _ = config::write_ui_prefs(&artnet::effective_dir(params), &st.prefs);
        }
        ui.label(RichText::new("(1 Hz = 60 BPM; Dateien speichern immer Hz)").weak().small());
    });
    ui.label(
        RichText::new("Größe: oben rechts (50–200 %), zoomt die komplette Oberfläche. Gilt wie Hz/BPM für alle Projekte (oberflaeche.txt).")
            .weak()
            .small(),
    );

    ui.add_space(10.0);
    ui.heading("Konfig-Ordner");
    ui.horizontal_wrapped(|ui| {
        ui.add(egui::TextEdit::singleline(&mut st.dir_edit).desired_width(420.0));
        if ui.button("Übernehmen").clicked() {
            if let Ok(mut d) = params.config_dir.write() {
                *d = st.dir_edit.trim().to_string();
            }
            shared.reload_request.store(true, Relaxed);
        }
        if ui.button("Standard").clicked() {
            if let Ok(mut d) = params.config_dir.write() {
                d.clear();
            }
            st.dir_edit = config::default_config_dir().display().to_string();
            shared.reload_request.store(true, Relaxed);
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui.button("Ordner öffnen").clicked() {
            open_path(&artnet::effective_dir(params));
        }
        if ui.button("Neu laden").clicked() {
            shared.reload_request.store(true, Relaxed);
        }
        if ui.button("Alle auf Ruhewert").clicked() {
            shared.reset_all_to_rest();
        }
        if ui.button("Reaper-Notennamen exportieren").clicked() {
            let r = config::export_note_names(show).map(|p| format!("Notennamen gespeichert in {}", p.display()));
            report(st, shared, r);
        }
    });

    ui.add_space(10.0);
    attribution(ui);
}
