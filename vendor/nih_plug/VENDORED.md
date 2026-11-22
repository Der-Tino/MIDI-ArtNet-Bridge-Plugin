# nih_plug (eingebundene Kopie)

Kopie des Kerns von https://github.com/robbert-vdh/nih-plug (Stand de42101), Lizenz: ISC (siehe LICENSE).
Mitkopiert: `nih_plug_derive`. Entfernt: Workspace-, Profil- und Dev-Abschnitte im Manifest.

Änderungen gegenüber dem Original – Größenänderung durch den Host
(z. B. Ziehen am Rand des Reaper-FX-Fensters), im Original als TODO offen:
- `Editor` hat zusätzlich `can_resize()`, `min_size()` und `set_size()` (mit Standardimplementierung).
- CLAP: `gui.can_resize`, `get_resize_hints`, `adjust_size`, `set_size` umgesetzt.
- VST3: `canResize`, `onSize`, `checkSizeConstraint` umgesetzt.
