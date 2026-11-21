//! GUI: Status, Master/Blackout, Konfig-Ordner, Live-Vorschau aller Geräte.

use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText};
use nih_plug_egui::{create_egui_editor, widgets::ParamSlider};

use crate::artnet;
use crate::config::{self, Level, PatchedFixture, Show, BLOCK_SIZE};
use crate::preview;
use crate::shared::{self, Shared, MAX_BUFS};
use crate::BridgeParams;

const RED: Color32 = Color32::from_rgb(230, 80, 70);
const YELLOW: Color32 = Color32::from_rgb(230, 190, 60);
const GREEN: Color32 = Color32::from_rgb(90, 200, 110);

#[derive(Default)]
struct UiState {
    dir_edit: String,
    initialized: bool,
    info: Option<(bool, String)>,
}

pub fn create(params: Arc<BridgeParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let egui_state = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        UiState::default(),
        |_ctx: &egui::Context, _st: &mut UiState| {},
        move |ctx: &egui::Context, setter: &ParamSetter, st: &mut UiState| {
            ctx.request_repaint();
            let show: Option<Arc<Show>> = shared.show.lock().ok().and_then(|g| g.clone());
            if !st.initialized {
                st.dir_edit = artnet::effective_dir(&params).display().to_string();
                st.initialized = true;
            }
            let master = master_value(&params);

            egui::CentralPanel::default().show(ctx, |ui| {
                header(ui, &params, &shared, setter, show.as_deref());
                ui.separator();
                folder_row(ui, &params, &shared, st, show.as_deref());
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match show.as_deref() {
                        Some(show) => {
                            messages(ui, show);
                            midi_table(ui, show);
                            ui.add_space(6.0);
                            fixtures(ui, show, &shared, master);
                        }
                        None => {
                            ui.label("Lade Konfiguration …");
                        }
                    });
            });
        },
    )
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

fn header(
    ui: &mut egui::Ui,
    params: &BridgeParams,
    shared: &Shared,
    setter: &ParamSetter,
    show: Option<&Show>,
) {
    ui.horizontal(|ui| {
        ui.heading("DMX Bridge");
        ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).weak());
    });

    ui.horizontal(|ui| {
        ui.label("Master");
        ui.add(ParamSlider::for_param(&params.master, setter));
        ui.add_space(12.0);
        toggle(ui, setter, &params.blackout, "Blackout");
        toggle(ui, setter, &params.output_enabled, "Art-Net Ausgabe");
    });

    let target = show
        .map(|s| format!("{}:{}", s.artnet_ip, s.artnet_port))
        .unwrap_or_else(|| "–".into());
    let unis: Vec<String> = (0..MAX_BUFS)
        .filter(|&b| shared.buf_active[b].load(Relaxed))
        .map(|b| shared.buf_universe[b].load(Relaxed).to_string())
        .collect();
    let sent = shared.packets_sent.load(Relaxed);
    let errors = shared.send_errors.load(Relaxed);
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "Ziel {target}  ·  Universen [{}]  ·  {sent} Pakete  ·  {errors} Sendefehler",
            unis.join(", ")
        ));
    });

    if !params.output_enabled.value() {
        ui.colored_label(YELLOW, "Ausgabe deaktiviert – es wird nichts gesendet.");
    } else if params.blackout.value() {
        ui.colored_label(RED, "BLACKOUT aktiv");
    }
    if let Some(e) = shared.net_error.lock().ok().and_then(|g| g.clone()) {
        ui.colored_label(RED, format!("Netzwerk: {e}"));
    }

    // letztes MIDI-Event – hilfreich zum Debuggen der Zuordnung
    let count = shared.event_count.load(Relaxed);
    let text = if count == 0 {
        "Noch kein MIDI empfangen.".to_string()
    } else {
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
            format!("Letztes MIDI: {kind_s} · Kanal {}", ch + 1)
        } else {
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
    };
    ui.label(RichText::new(text).monospace().small());
}

fn folder_row(
    ui: &mut egui::Ui,
    params: &BridgeParams,
    shared: &Shared,
    st: &mut UiState,
    show: Option<&Show>,
) {
    ui.horizontal(|ui| {
        ui.label("Konfig-Ordner:");
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
    ui.horizontal(|ui| {
        if ui.button("Ordner öffnen").clicked() {
            open_path(&artnet::effective_dir(params));
        }
        if ui.button("Neu laden").clicked() {
            shared.reload_request.store(true, Relaxed);
            st.info = None;
        }
        if ui.button("Alle auf Ruhewert").clicked() {
            shared.reset_all_to_rest();
        }
        if ui.button("Reaper-Notennamen exportieren").clicked() {
            if let Some(show) = show {
                st.info = Some(match config::export_note_names(show) {
                    Ok(p) => (true, format!("Notennamen gespeichert in {}", p.display())),
                    Err(e) => (false, format!("Export fehlgeschlagen: {e}")),
                });
            }
        }
    });
    if let Some((ok, txt)) = &st.info {
        ui.colored_label(if *ok { GREEN } else { RED }, txt.as_str());
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

fn messages(ui: &mut egui::Ui, show: &Show) {
    for m in &show.messages {
        match m.level {
            Level::Error => ui.colored_label(RED, format!("Fehler: {}", m.text)),
            Level::Warn => ui.colored_label(YELLOW, format!("Hinweis: {}", m.text)),
            Level::Info => ui.label(RichText::new(m.text.as_str()).weak()),
        };
    }
}

fn midi_table(ui: &mut egui::Ui, show: &Show) {
    egui::CollapsingHeader::new("MIDI-Kanal → Universum / Block")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("midimap").striped(true).show(ui, |ui| {
                for (ch, t) in show.midi.iter().enumerate() {
                    ui.label(format!("MIDI {}", ch + 1));
                    match t {
                        Some(t) => {
                            ui.label(format!("Universum {}", t.universe));
                            ui.label(format!(
                                "Block {}  ·  DMX {}–{}",
                                t.block + 1,
                                t.first_slot() + 1,
                                t.first_slot() + BLOCK_SIZE
                            ));
                        }
                        None => {
                            ui.label(RichText::new("aus").weak());
                            ui.label("");
                        }
                    }
                    ui.end_row();
                }
            });
        });
}

fn fixtures(ui: &mut egui::Ui, show: &Show, shared: &Shared, master: f32) {
    if show.fixtures.is_empty() {
        ui.heading("Geräte");
        ui.label("Keine Geräte gepatcht – patch.txt bearbeiten.");
        return;
    }
    let t = ui.input(|i| i.time);

    // aktuelle Werte (inkl. Master) einmal pro Frame einsammeln
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

    // ---- Bühnenansicht: eine Kachel pro Gerät ----
    ui.heading("Bühne");
    ui.horizontal_wrapped(|ui| {
        for (f, vals) in show.fixtures.iter().zip(&all_vals) {
            let p = preview::compute(f, vals, t);
            ui.vertical(|ui| {
                ui.set_max_width(70.0);
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(64.0, 40.0), egui::Sense::hover());
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
                        format!("{hz:.0} Hz"),
                        egui::FontId::proportional(10.0),
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
    ui.heading("Geräte & Fader");
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
                .default_open(false)
                .show(ui, |ui| {
                    if buf.is_none() {
                        ui.colored_label(YELLOW, "Universum ist keinem MIDI-Kanal zugeordnet – wird nicht gesendet.");
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
                            ui.label(match ch.cap_at(out) {
                                Some(cap) => cap.describe(out),
                                None => String::new(),
                            });
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
