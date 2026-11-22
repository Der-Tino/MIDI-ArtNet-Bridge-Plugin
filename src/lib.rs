//! DMX Bridge – MIDI-Noten in Reaper direkt als DMX über Art-Net.
//!
//! MIDI-Kanal → Universum + Block (je 128 Slots, wie die X32-Input-Blocks)
//! Note n      → Slot n im Block
//! Velocity    → DMX-Wert (1–127 → 2–255), Note-Off → Ruhewert
//! CC n        → Slot n, Wert dauerhaft (für Fades per CC-Lane)
//! Poly-AT     → Wert einer gehaltenen Note ändern

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::num::NonZeroU32;
use std::sync::{Arc, RwLock};

mod artnet;
mod config;
mod editor;
pub mod i18n;
mod inflate;
mod library;
mod preview;
mod qxf;
mod shared;

use shared::Shared;

pub struct DmxBridge {
    params: Arc<BridgeParams>,
    shared: Arc<Shared>,
    sender: Option<artnet::Sender>,
}

#[derive(Params)]
pub struct BridgeParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    /// leer = Standardordner (Dokumente/DMX-Bridge)
    #[persist = "config-dir"]
    pub config_dir: RwLock<String>,

    /// wirkt auf alle Dimmer-/Farbkanäle, automatisierbar
    #[id = "master"]
    pub master: FloatParam,

    #[id = "blackout"]
    pub blackout: BoolParam,

    /// aus = gar nichts senden (z. B. wenn ein Lichtpult übernimmt)
    #[id = "output"]
    pub output_enabled: BoolParam,
}

impl Default for BridgeParams {
    fn default() -> Self {
        Self {
            editor_state: {
                let (w, h) = editor::window_size_for(1.0);
                EguiState::from_size(w, h)
            },
            config_dir: RwLock::new(String::new()),
            master: FloatParam::new("Master", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            blackout: BoolParam::new("Blackout", false),
            output_enabled: BoolParam::new("Art-Net Ausgabe", true),
        }
    }
}

impl Default for DmxBridge {
    fn default() -> Self {
        Self {
            params: Arc::new(BridgeParams::default()),
            shared: Arc::new(Shared::default()),
            sender: None,
        }
    }
}

impl Plugin for DmxBridge {
    const NAME: &'static str = "DMX Bridge";
    const VENDOR: &'static str = "DMX Node";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Audio wird unverändert durchgereicht; das Plugin sitzt auf einer MIDI-Spur.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        // Größe (Zoom) aus oberflaeche.txt schon vor dem ersten Öffnen setzen,
        // damit das Fenster direkt in der richtigen Größe aufgeht.
        let prefs = config::read_ui_prefs(&artnet::effective_dir(&self.params));
        i18n::set_english(prefs.english);
        nih_plug_egui::set_ui_zoom(editor::zoom_for(prefs.scale));
        self.params
            .editor_state
            .set_size(editor::window_size_for(prefs.scale));
        editor::create(self.params.clone(), self.shared.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        _buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        // Projekt-State (Konfig-Ordner) ist jetzt geladen → neu einlesen
        self.shared
            .reload_request
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if self.sender.is_none() {
            self.sender = Some(artnet::Sender::start(
                self.shared.clone(),
                self.params.clone(),
            ));
        }
        true
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn {
                    channel,
                    note,
                    velocity,
                    ..
                } => self.shared.note_on(channel, note, velocity),
                NoteEvent::NoteOff { channel, note, .. } => self.shared.note_off(channel, note),
                NoteEvent::PolyPressure {
                    channel,
                    note,
                    pressure,
                    ..
                } => self.shared.pressure(channel, note, pressure),
                NoteEvent::MidiCC {
                    channel, cc, value, ..
                } => self.shared.cc(channel, cc, value),
                _ => {}
            }
        }
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for DmxBridge {
    const CLAP_ID: &'static str = "de.dmx-node.dmx-bridge";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("MIDI-Noten als DMX über Art-Net, Fixtures per Textdatei");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Utility];
}

impl Vst3Plugin for DmxBridge {
    const VST3_CLASS_ID: [u8; 16] = *b"DmxBridgeArtNet2";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Tools];
}

nih_export_clap!(DmxBridge);
nih_export_vst3!(DmxBridge);
