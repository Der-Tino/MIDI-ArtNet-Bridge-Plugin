<img width="2400" height="800" alt="A · Wordmark dark@2x" src="https://github.com/user-attachments/assets/d0d78ad8-5b78-4847-9c66-7ca03c8562fc" />


# MIDI ArtNet Bridge – MIDI → Art-Net as a CLAP/VST3 plugin

*[Deutsche Version](README.de.md)*

[![GitHub Sponsors](https://img.shields.io/badge/Sponsor-GitHub-ea4aaa?logo=githubsponsors&logoColor=white)](https://github.com/sponsors/Der-Tino) [![Ko-fi](https://img.shields.io/badge/Support-Ko--fi-FF5E5B?logo=ko-fi&logoColor=white)](https://ko-fi.com/dertino)

Program your light show as MIDI notes in your DAW (built and tested for **Reaper**) –
the plugin sends the values straight out as **Art-Net**. No QLC+, no virtual MIDI
cable, no background script.

- MIDI channel → DMX universe + block, 16 channels = 4 full universes
- Note = DMX channel, velocity = value; CC lanes for fades
- Fixtures as simple text files or directly as QLC+ `.qxf` files
- Built-in **fixture library** (~1800 fixtures, mostly from QLC+), searchable by manufacturer
- Patch and fixture editor, live stage preview incl. strobe, faders per channel
- Art-Net node discovery (ArtPoll), master/blackout as automatable parameters

> **Version 1.0.** The user interface is available in English and German (defaults to your system language, switch top right).

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
can be edited inside the plugin (tabs *Patch*, *Custom Fixtures*, *Fixture Library*,
*Settings*) or by hand in any text editor – **saved changes are picked up
automatically** within about a second. To move your setup to another computer, copy
the whole folder.

```
DMX-Bridge/
├── config.txt          Art-Net target IP, refresh rate, MIDI channel → universe/block
├── patch.txt           name | fixture | universe | start address
├── fixtures/           one fixture type per file (.txt or QLC+ .qxf)
├── ui.txt              UI size, language, Hz/BPM, note display (global, all projects)
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

The target IP is the **node's** IP, not your computer's. The *Settings* tab shows
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
Washer | generic_wall_washer_rgbw | 0 | 1
PAR    | generic_par_rgbwauv      | 0 | 9   | count=12          ← PAR 1 … PAR 12 in a row
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
5. *Settings → Export Reaper note names* writes note-name files; load them in
   the MIDI editor (*File → Note names → Load note names from file*) to see
   “Washer: Red” instead of “C#-1”.

## Window size and zoom

- **Zoom** (50–200 %) top right – the whole interface scales, proportions stay the same.
- **Window size**: drag the grip in the bottom-right corner, or simply drag the edge of
  Reaper's FX window. Double-click the grip for the default size.
- Zoom, window size, language, Hz/BPM and note display are stored in `ui.txt` and apply to
  all projects.

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
*Actions → Build → Run workflow* with a release tag (e.g. `v1.0.0`) creates a release.

## Support the project

Found a bug? *Settings → About → Report a bug* opens a pre-filled GitHub issue.
If you like the plugin, you can support its development via
[**GitHub Sponsors**](https://github.com/sponsors/Der-Tino) or [**Ko-fi**](https://ko-fi.com/dertino)
(no GitHub account needed) – also reachable via the ♥ button at the top right of the plugin.

## License

MIDI ArtNet Bridge – Copyright (C) 2026 Der-Tino and contributors

This program is free software: you can redistribute it and/or modify it under the terms
of the **GNU General Public License** as published by the Free Software Foundation,
either **version 3** of the License, or (at your option) any later version.
It is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY.
See [`LICENSE`](LICENSE) for the full text.

In short: everyone may use, study, modify and share it for free. Anyone who distributes
it – modified or not – must do so under the same license and include the source code.

## Third-party components

| Component | License | Notes |
|---|---|---|
| [QLC+](https://www.qlcplus.org) fixture definitions | Apache 2.0 | `library/` – embedded, unmodified; update with `tools/update_library.py` |
| Own fixture definitions | GPL-3.0-or-later | `library/custom/` – e.g. a 9 × 15 W RGBW wall washer |
| [NIH-plug](https://github.com/robbert-vdh/nih-plug) | ISC | plugin framework |
| [vst3-sys](https://github.com/RustAudio/vst3-sys) | GPLv3 | VST3 bindings used for the VST3 build |
| `vendor/nih_plug_egui` | ISC | modified copy, see `VENDORED.md` |
| `vendor/egui-baseview` | MIT | modified copy (UI zoom), see `VENDORED.md` |
