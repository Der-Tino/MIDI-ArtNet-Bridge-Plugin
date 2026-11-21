# nih_plug_egui (eingebundene Kopie)

Kopie von `nih_plug_egui` aus https://github.com/robbert-vdh/nih-plug (Stand de42101, 2026-09-03),
Lizenz: ISC (siehe LICENSE).

Änderungen gegenüber dem Original:
- `EguiState::set_requested_size` ist `pub`, damit das Plugin die Fenstergröße
  beim Wechsel der Skalierung (50–200 %) selbst setzen kann.
- `nih_plug` wird per Git statt per relativem Pfad eingebunden.
