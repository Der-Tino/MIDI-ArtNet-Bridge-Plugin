MIDI-Noten in Reaper direkt als DMX über Art-Net. Kein QLC+ und kein virtuelles MIDI-Kabel nötig.

> ⚠️ Vorabversionen (alpha/beta) sind nicht live erprobt. Vor einem Gig bitte ausgiebig testen.

### Downloads
| System | Datei |
|---|---|
| Windows | `DMX-Bridge-windows.zip` |
| macOS (Intel + Apple Silicon) | `DMX-Bridge-macos-universal.zip` |
| Linux | `DMX-Bridge-linux.zip` |

### Installation
- **Windows:** `DMX Bridge.clap` → `C:\Program Files\Common Files\CLAP\`
- **macOS:** `DMX Bridge.clap` → `~/Library/Audio/Plug-Ins/CLAP/`, danach einmalig im Terminal:
  `xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins/CLAP/"DMX Bridge.clap"`
- In Reaper: Plugin auf die Licht-MIDI-Spur legen und für diese Spur **Anticipative FX abschalten**.

### Funktionen
- MIDI-Kanal → Universum + Block (je 128 Kanäle, wie die X32-Input-Blocks), 16 Kanäle = 4 volle Universen
- Note = DMX-Kanal, Velocity = Wert, Note-Off = Ruhewert; CC-Lanes für Fades, Poly-Aftertouch
- Fixtures als Textdateien oder direkt als QLC+-`.qxf`, mit Modi; Patch mit `anzahl=` für viele gleiche Lampen
- Alle gängigen Kanaltypen (RGBWA+UV, CMY, Farbrad, Pan/Tilt, Gobo, Prisma …) und Wertebereiche inkl. Strobe in Hz
- Live-Vorschau mit Mischfarbe und blinkendem Strobe, Fader pro Kanal
- Master, Blackout und Ausgabe-Schalter als automatisierbare Parameter
- Konfig wird beim Speichern automatisch neu geladen
- Export von Reaper-Notennamen („PAR 1: Rot“ statt „C#-1“)

Details siehe [README](https://github.com/Der-Tino/MIDI-DMX-Bridge-CLAP#readme).
