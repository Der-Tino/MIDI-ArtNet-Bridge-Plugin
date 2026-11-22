# egui-baseview (eingebundene Kopie)

Kopie von https://github.com/BillyDM/egui-baseview (Rev ec70c3f, 2025-02-04), Lizenz: MIT (siehe LICENSE).

Änderungen gegenüber dem Original:
- Globaler Oberflächen-Zoom (`set_ui_zoom`, `ui_zoom`): Pixel pro Punkt = System-Skalierung × Zoom.
  Das Layout (in Punkten) bleibt gleich, alles – Schrift, Grafik, Mauskoordinaten – skaliert mit.
- Startgröße des Fensters wird korrekt in physische Pixel umgerechnet.
