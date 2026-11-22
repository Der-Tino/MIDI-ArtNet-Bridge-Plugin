MIDI notes in your DAW → DMX over Art-Net, as a CLAP/VST3 plugin.
*(Deutsch: siehe README.de.md im Repository.)*

> ⚠️ Pre-releases (alpha/beta) are not tested live yet – test thoroughly before a gig.

### Downloads
| System | File |
|---|---|
| Windows | `DMX-Bridge-windows.zip` |
| macOS (Intel + Apple Silicon) | `DMX-Bridge-macos-universal.zip` |
| Linux | `DMX-Bridge-linux.zip` |

### Installation
- **Windows:** `DMX Bridge.clap` → `C:\Program Files\Common Files\CLAP\`
- **macOS:** `DMX Bridge.clap` → `~/Library/Audio/Plug-Ins/CLAP/`, then once in Terminal:
  `xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins/CLAP/"DMX Bridge.clap"`
- **Linux:** `DMX Bridge.clap` → `~/.clap/`
- In Reaper: put the plugin on your light MIDI track and **disable anticipative FX** for that track.
  If you cannot type into fields, enable **“Send all keyboard input to plugin”** in the FX window.

Full documentation: [README](REPO_URL#readme)
