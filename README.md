# MIDI ArtNet Bridge – MIDI → Art-Net as a CLAP/VST3 plugin

*[Deutsche Version](README.de.md)*

Program your light show as MIDI notes in your DAW (built and tested for **Reaper**) –
the plugin sends the values straight out as **Art-Net**. No QLC+, no virtual MIDI
cable, no background script.

- MIDI channel → DMX universe + block, 16 channels = 4 full universes
- Note = DMX channel, velocity = value; CC lanes for fades
- Fixtures as simple text files or directly as QLC+ `.qxf` files
- Built-in **QLC+ fixture library** (~1800 fixtures), searchable by manufacturer
- Patch and fixture editor, live stage preview incl. strobe, faders per channel
- Art-Net node discovery (ArtPoll), master/blackout as automatable parameters

> **Status: alpha.** The user interface is currently in German; an English UI is planned.

## Download

Ready-to-use plugins for Windows, macOS (Intel + Apple Silicon) and Linux are on the
**Releases** page. Unzip and copy `DMX Bridge.clap` into your CLAP folder:

| System  | CLAP folder                                  |
|---------|----------------------------------------------|
| Windows | `C:\Program Files\Common Files\CLAP\`        |
| macOS   | `~/Library/Audio/Plug-Ins/CLAP/`             |
| Linux   | `~/.clap/`                                   |

On macOS, remove the quarantine flag once (the plugin is not signed):

```bash
xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins/CLAP/"DMX Bridge.clap"
```

Prefer the **CLAP** version in Reaper (MIDI CCs arrive more reliably than with VST3).

## How it works

```
MIDI channel →  universe + block   (config.txt, like the input blocks on an X32)
Note 0–127   →  slot in the block  (note 0 = first channel of the block)
Velocity     →  DMX value          (1–127 → 2–255, 127 = full)
Note off     →  rest value         (from the fixture file, default 0)
CC 0–119     →  same slot as the note, value is held → for fades
Poly AT      →  changes the value of a held note
```

Default mapping (freely configurable):

| MIDI channel | Universe | Block | DMX      |
|-------------:|---------:|------:|----------|
| 1            | 0        | 1     | 1–128    |
| 2            | 0        | 2     | 129–256  |
| 3            | 0        | 3     | 257–384  |
| 4            | 0        | 4     | 385–512  |
| 5–8          | 1        | 1–4   | as above |
| 9–16         | off      |       |          |

## Configuration folder

On first start the plugin creates `Documents/DMX-Bridge/` with example files. Everything
can be edited inside the plugin (tabs *Patch*, *Fixtures*, *QLC+ Bibliothek*,
*Einstellungen*) or by hand in any text editor – **saved changes are picked up
automatically** within about a second. To move your setup to another computer, copy
the whole folder.

```
DMX-Bridge/
├── config.txt          Art-Net target IP, refresh rate, MIDI channel → universe/block
├── patch.txt           name | fixture | universe | start address
├── fixtures/           one fixture type per file (.txt or QLC+ .qxf)
├── oberflaeche.txt     UI size and Hz/BPM (global, all projects)
└── reaper-notennamen/  Reaper note-name files (created on request)
```

### config.txt

```
artnet_ip   = 192.168.0.50     # IP of your Art-Net node (255.255.255.255 = broadcast)
artnet_port = 6454
refresh_hz  = 40
cc_input    = on
midi1       = 0 1              # MIDI channel 1 → universe 0, block 1 (DMX 1–128)
midi2       = 0 2
midi3       = off
```

The target IP is the **node's** IP, not your computer's. The *Einstellungen* tab shows
which IP your computer sends from, warns if the node is in a different subnet, and can
search the network for nodes.

### Fixture files

`fixtures/my_par.txt` (file name = fixture name used in the patch):

```
name = My PAR
# channel | type | label | rest value
1 | dimmer | Dimmer | 255
2 | red    | Red
3 | green  | Green
4 | blue   | Blue
5 | white  | White
6 | amber  | Amber
7 | uv     | UV
8 | strobe | Strobe
  0-9    | open        | no strobe        ← value ranges for the channel above
  10-255 | strobe 1-25 | strobe 1–25 Hz
```

**Channel types:** `dimmer, dimmer_fine, red, green, blue, white, amber, uv, lime, indigo,
cyan, magenta, yellow, colorwheel, colormacro, cto, strobe, pan, pan_fine, tilt, tilt_fine,
pt_speed, gobo, gobo_rotation, prism, prism_rotation, focus, zoom, iris, frost, rotation,
effect, speed, fog, maintenance, other` (German keywords work as well).

**Value ranges** (`from-to | function | label`), equivalent to QLC+ capabilities:

| Function               | Meaning                                              |
|------------------------|------------------------------------------------------|
| `open`                 | light on, no strobe                                  |
| `closed`               | shutter closed, dark                                 |
| `strobe 1-25`          | strobe across the range, 1 → 25 Hz (also `strobe 60-1500 bpm`) |
| `random 1-20`          | random strobe                                        |
| `color #ff8000`        | colour on a colour wheel/macro (also `color red`)    |
| `color red green`      | split colour                                         |
| `off`                  | no function                                          |
| anything else          | free text (gobo 3, auto program …)                   |

A strobe channel without ranges defaults to 0–9 open, 10–255 = 1–25 Hz – check the
threshold on your fixture.

QLC+ fixture files (`.qxf`) can simply be dropped into `fixtures/`; channels, modes,
default values and capabilities (incl. strobe Hz and colours) are imported.

### patch.txt

```
Washer | uking_wash_8ch  | 0 | 1
PAR    | par_rgbwauv_8ch | 0 | 9   | count=12          ← PAR 1 … PAR 12 in a row
Spot   | my_spot         | 0 | 105 | mode=8 Channel    ← mode of a .qxf fixture
```

Optional columns: `mode=…`, `count=n` (or `x12`), `spacing=n` (address step,
default = channel count).

## Reaper setup

1. Put the plugin on your light MIDI track. **One instance per project.**
2. **Disable anticipative FX for that track** (right-click the track → performance
   options → *Prevent anticipative FX*). Otherwise Reaper renders ahead and the light
   is ~200 ms early.
3. Enable *Preferences → Playback → Chase MIDI note-ons*, so lights come on when you
   start playback in the middle of a long note.
4. If you cannot type into text or number fields: in the FX window enable
   **“Send all keyboard input to plugin”**.
5. *Einstellungen → Reaper-Notennamen exportieren* writes note-name files; load them in
   the MIDI editor (*File → Note names → Load note names from file*) to see
   “Washer: Red” instead of “C#-1”.

## Fades

- **CC lane** (recommended): open a CC lane with the same number as the note and draw
  a curve – Reaper's curve shapes work directly. Works for slots 0–119 of each block.
- **Poly aftertouch** changes the value of a held note.
- Latest event wins (LTP).

The *Live* tab has a strobe calculator per strobe channel: enter a frequency (Hz or BPM)
and get the matching DMX value and MIDI velocity.

## Robustness

- The audio thread only writes atomics – no locks, no allocations.
- Art-Net runs in its own thread with a 40 Hz keep-alive; changes go out immediately
  (max. ~44 Hz per universe).
- Network errors (cable unplugged, node rebooted) are recovered automatically.
- Master and blackout are plugin parameters and can be automated or mapped to a controller.

## Building from source

```bash
# Rust: https://rustup.rs
cargo xtask bundle dmx_bridge --release
# macOS universal binary (Intel + Apple Silicon):
cargo xtask bundle-universal dmx_bridge --release
```

Output: `target/bundled/`. GitHub Actions builds Windows, macOS and Linux on every push;
*Actions → Build → Run workflow* with a release tag (e.g. `v0.5.0`) creates a release.

## Third-party components

| Component | License | Notes |
|---|---|---|
| [QLC+](https://www.qlcplus.org) fixture definitions | Apache 2.0 | `library/` – embedded, unmodified; update with `tools/update_library.py` |
| [NIH-plug](https://github.com/robbert-vdh/nih-plug) | ISC | plugin framework |
| [vst3-sys](https://github.com/RustAudio/vst3-sys) | GPLv3 | VST3 bindings used for the VST3 build |
| `vendor/nih_plug_egui` | ISC | modified copy, see `VENDORED.md` |
| `vendor/egui-baseview` | MIT | modified copy (UI zoom), see `VENDORED.md` |
