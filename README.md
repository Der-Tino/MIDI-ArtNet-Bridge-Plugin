# DMX Bridge v0.4 – MIDI → Art-Net als CLAP/VST3

Licht als MIDI-Noten in Reaper, das Plugin schickt die Werte direkt als Art-Net raus.
Kein QLC+, kein virtuelles MIDI-Kabel, kein Hintergrund-Skript.

## Bauen

```bash
# einmalig: Rust installieren (https://rustup.rs)
cargo xtask bundle dmx_bridge --release
# Mac (Intel + Apple Silicon in einer Datei):
cargo xtask bundle-universal dmx_bridge --release
```

Ergebnis in `target/bundled/`: `DMX Bridge.clap` und `DMX Bridge.vst3`.

| System  | CLAP-Ordner                                  |
|---------|----------------------------------------------|
| Windows | `C:\Program Files\Common Files\CLAP\`        |
| macOS   | `~/Library/Audio/Plug-Ins/CLAP/`             |

Nimm in Reaper die **CLAP**-Version (MIDI-CCs kommen dort zuverlässiger an als bei VST3).

## Das Prinzip

```
MIDI-Kanal  →  Universum + Block   (in config.txt, wie die Input-Blocks am X32)
Note 0–127  →  Slot im Block       (Note 0 = erster Kanal des Blocks)
Velocity    →  DMX-Wert            (1–127 → 2–255, 127 = voll)
Note-Off    →  Ruhewert            (aus der Fixture-Datei, Standard 0)
CC 0–119    →  gleicher Slot wie die Note, Wert bleibt stehen → für Fades
Poly-AT     →  ändert den Wert einer gehaltenen Note
```

Standard-Zuordnung (frei änderbar):

| MIDI-Kanal | Universum | Block | DMX      |
|-----------:|----------:|------:|----------|
| 1          | 0         | 1     | 1–128    |
| 2          | 0         | 2     | 129–256  |
| 3          | 0         | 3     | 257–384  |
| 4          | 0         | 4     | 385–512  |
| 5–8        | 1         | 1–4   | wie oben |
| 9–16       | aus       |       |          |

16 MIDI-Kanäle × 128 Noten = 2048 Slots = 4 komplette Universen.

## Konfig-Ordner

Beim ersten Start legt das Plugin `Dokumente/DMX-Bridge/` mit Beispielen an.
Den Pfad kannst du im Plugin ändern; er wird im Reaper-Projekt gespeichert.
**Gespeicherte Änderungen werden automatisch übernommen** (nach ca. 1 s).

```
DMX-Bridge/
├── config.txt            Art-Net-IP, Rate, MIDI-Kanal → Universum/Block
├── patch.txt             Name | Fixture | Universum | Startadresse
├── fixtures/
│   ├── uking_wash_8ch.txt
│   ├── par_rgbwauv_8ch.txt
│   ├── beispiel_movinghead.txt
│   ├── rgbw_4ch.txt
│   ├── dimmer_1ch.txt
│   └── (deine .qxf aus QLC+)
└── reaper-notennamen/    (wird per Knopf erzeugt)
```

Zum Mitnehmen auf ein anderes Gerät kopierst du einfach den ganzen Ordner.

### Fixtures – eigenes Textformat

`fixtures/meinpar.txt` (Dateiname = Name im Patch):

```
name = Mein PAR
# Kanal | Typ | Bezeichnung | Ruhewert
1 | dimmer | Dimmer | 255
2 | rot    | Rot
3 | grün   | Grün
4 | blau   | Blau
5 | weiß   | Weiß
6 | amber  | Amber
7 | uv     | UV
8 | strobe | Strobe
  0-9    | offen       | kein Strobe        ← Wertebereiche für den Kanal darüber
  10-255 | strobe 1-25 | Strobe 1–25 Hz
```

**Kanaltypen:** `dimmer, dimmer_fein, rot, grün, blau, weiß, amber, uv, lime, indigo,
cyan, magenta, gelb, farbrad, farbmakro, cto, strobe, pan, pan_fein, tilt, tilt_fein,
pt_speed, gobo, gobo_rotation, prisma, prisma_rotation, fokus, zoom, iris, frost,
rotation, effekt, speed, nebel, wartung, sonst`

**Wertebereiche** (`von-bis | Funktion | Text`), entsprechen den Capabilities in QLC+:

| Funktion             | Bedeutung                                      |
|----------------------|------------------------------------------------|
| `offen`              | Licht an, kein Strobe                          |
| `zu`                 | Shutter zu, dunkel                             |
| `strobe 1-25`        | Strobe, über den Bereich 1 → 25 Hz (auch `strobe 60-1500 bpm`) |
| `zufall 1-20`        | Zufalls-Strobe                                 |
| `farbe #ff8000`      | Farbe auf Farbrad/Makro (auch `farbe rot`)     |
| `farbe rot grün`     | geteilte Farbe                                 |
| `aus`                | keine Funktion                                 |
| alles andere         | freier Text (Gobo 3, Auto-Programm …)          |

Ein Strobe-Kanal ohne eigene Bereiche bekommt automatisch: 0–9 offen,
10–255 = 1–25 Hz. **Die Schwelle (10) bitte an deiner Lampe prüfen.**

### Fixtures – direkt aus QLC+ (.qxf)

Kopiere deine QLC+-Fixture-Dateien (Windows: `C:\Users\<du>\QLC+\Fixtures\`)
einfach in `fixtures/`. Übernommen werden Kanäle, Modi, Default-Werte und Capabilities
inkl. Strobe-Hz und Farben. Das alte und das neue QLC+-Format werden erkannt.

### Patch

```
Washer   | uking_wash_8ch  | 0 | 1
PAR      | par_rgbwauv_8ch | 0 | 9  | anzahl=12          ← PAR 1 … PAR 12 am Stück
Washer Q | meine_washer    | 0 | 105 | modus=8 Channel   ← Modus einer .qxf
```

Optionale Spalten: `modus=…` (oder einfach der Modusname), `anzahl=n` (oder `x12`),
`abstand=n` (Adressabstand, Standard = Kanalanzahl).

Fehler (Tippfehler, Überschneidungen, Geräte ohne MIDI-Zuordnung) zeigt das Plugin
gelb/rot an. Es stürzt deswegen nicht ab, und die restliche Konfiguration läuft weiter.

## Reaper-Setup

1. Plugin auf die Licht-MIDI-Spur legen. **Nur eine Instanz pro Projekt.**
2. **Anticipative FX für diese Spur abschalten** (Rechtsklick auf die Spur →
   Performance-Optionen → *Prevent anticipative FX*). Sonst rechnet Reaper die Spur
   voraus, und das Licht kommt rund 200 ms zu früh.
3. *Preferences → Playback → Chase MIDI note-ons* einschalten. Dann geht das Licht
   auch beim Start mitten in einer langen Note an.
4. Beim Stop sendet Reaper „All Notes Off“: Alle Slots gehen auf ihren Ruhewert.
   Falls Reaper beim Stop zusätzlich CCs zurücksetzt und dabei Werte verstellt:
   in `config.txt` `cc_eingang = aus` setzen oder das CC-Reset in den Preferences abschalten.
5. Im Plugin **„Reaper-Notennamen exportieren“** drücken. Im MIDI-Editor dann die
   passende Datei aus `reaper-notennamen/` laden
   (*File → Note names → Load note names from file*). Statt „C#-1“ steht dann
   „Washer: Rot“ im Piano-Roll.

## GUI

Die Größe des ganzen Fensters wählst du oben rechts (50, 75, 100, 125, 150, 200 %) –
alles skaliert gleichmäßig mit. Größe und Hz/BPM gelten für alle Projekte
(`oberflaeche.txt` im Konfig-Ordner).

> **Tippen in Eingabefelder geht nicht?** Reaper gibt die Tastatur sonst nicht an das
> Plugin weiter: im FX-Fenster die Option **„Send all keyboard input to plugin“** einschalten.

| Tab | Was |
|---|---|
| **Live** | Bühnenansicht, Fader pro Kanal, Konfig-Meldungen |
| **Patch** | Geräte hinzufügen, entfernen, umsortieren; Fixture, Modus, Universum, Adresse, Anzahl |
| **Fixtures** | eigene Fixtures anlegen und bearbeiten (Kanäle, Typen, Ruhewerte, Wertebereiche); QLC+-Fixtures als bearbeitbare Kopie |
| **QLC+ Bibliothek** | alle ~1800 Fixtures aus QLC+, nach Herstellern sortiert und durchsuchbar; Modus wählen, mit einem Klick patchen |
| **Einstellungen** | Node-IP (mit Node-Suche per ArtPoll), Port, Rate, MIDI-Kanal → Universum/Block, Hz/BPM, Konfig-Ordner |

Alles, was du im Plugin speicherst, landet in denselben Textdateien (vorherige Version
jeweils als `.bak`). Handbearbeitung bleibt möglich.


- **Bühne:** eine Kachel pro Gerät mit Live-Farbe (RGBWA+UV, CMY, Farbrad/Makro × Dimmer).
  Strobe blinkt mit der eingestellten Frequenz, die Hz-Zahl steht in der Kachel.
  Shutter „zu“ wird dunkel dargestellt. Die Vorschau ist eine Annäherung: Ab etwa
  20 Hz kommt die Bildrate der GUI nicht mehr ganz mit.
- **Hz oder BPM:** unter Einstellungen umschaltbar. An jedem Strobe-Kanal gibt es einen
  Rechner: Wunschfrequenz eingeben → passender DMX-Wert und die MIDI-Velocity für Reaper.
- **Geräte & Fader:** pro Kanal ein Fader zum Testen, dazu die aktuelle Funktion
  („Strobe · 12.6 Hz“, „Farbrad: Rot“) und die MIDI-Note. Ein Fader-Wert gilt bis zum
  nächsten MIDI-Event auf diesem Kanal. „Alle auf Ruhewert“ setzt alles zurück.
- **Master/Blackout:** Hat ein Gerät einen Dimmer, wird nur der Dimmer heruntergezogen,
  damit die Farben sauber bleiben. Ohne Dimmer werden die Farbkanäle skaliert.

## Fades

- **CC-Lane** (empfohlen): Im MIDI-Editor eine CC-Lane mit derselben Nummer wie
  die Note öffnen und eine Kurve zeichnen. Reapers Kurvenformen (linear, slow
  start …) funktionieren direkt. Gilt für die Slots 0–119 jedes Blocks.
- **Poly-Aftertouch**: Ändert den Wert einer gehaltenen Note.
- Eine Note setzt den Wert und hält ihn bis Note-Off. Ein CC setzt den Wert
  dauerhaft bis zum nächsten Event. Es gilt immer das letzte Event (LTP).

## Robustheit

- Der Audio-Thread schreibt nur atomare Werte: keine Locks, keine Allokationen.
- Art-Net läuft in einem eigenen Thread mit 40 Hz Keep-Alive. Änderungen gehen
  sofort raus, aber nie schneller als ca. 44 Hz pro Universum.
- Netzwerkfehler (Kabel raus, Node neu gestartet): Der Socket wird automatisch neu aufgebaut.
- „Art-Net Ausgabe“ aus = es wird gar nichts gesendet (z. B. wenn die Haus-Lichtanlage übernimmt).
- Master und Blackout sind Plugin-Parameter und lassen sich in Reaper automatisieren
  oder auf einen Controller legen.

## QLC+-Fixture-Bibliothek

Die Fixture-Definitionen stammen aus dem Projekt [QLC+](https://www.qlcplus.org)
(Massimo Callegari und Mitwirkende) und stehen unter der Apache License 2.0
(`library/QLCPLUS-LICENSE.txt`). Sie sind komprimiert im Plugin eingebettet
(`library/qlcplus-fixtures.pack`). Aktualisieren:

```bash
git clone --depth 1 https://github.com/mcallegari/qlcplus /tmp/qlcplus
python3 tools/update_library.py /tmp/qlcplus
```
