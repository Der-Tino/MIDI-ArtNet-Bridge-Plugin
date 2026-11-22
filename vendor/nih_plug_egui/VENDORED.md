# nih_plug_egui (eingebundene Kopie)

Kopie von `nih_plug_egui` aus https://github.com/robbert-vdh/nih-plug (Stand de42101, 2026-09-03),
Lizenz: ISC (siehe LICENSE).

Änderungen gegenüber dem Original:
- `EguiState::set_requested_size` ist `pub`, damit das Plugin die Fenstergröße
  beim Wechsel der Skalierung (50–200 %) selbst setzen kann.
- `nih_plug` wird per Git statt per relativem Pfad eingebunden.
- Nutzt die eingebundene Kopie von egui-baseview (Zoom) mit `windows_keyboard_workaround`
  (holt den Tastaturfokus ins Plugin, sobald ein Eingabefeld aktiv ist).
- Fenstergrößenänderung übergibt keine logischen Werte mehr als physische Pixel.
- `EguiState::set_size` öffentlich; `set_ui_zoom`/`ui_zoom` werden re-exportiert.
- Größenänderung: angeforderte Größe bleibt gesetzt, bis der Host sie über size() abgefragt hat.
