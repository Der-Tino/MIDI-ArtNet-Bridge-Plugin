# TODO / Roadmap

## Done

- [x] **Resize grip (FabFilter style)** in the bottom-right corner: drag to resize, the zoom
      level stays; double-click or *Settings → Display → Default size* to reset. The window
      size is stored in `ui.txt` (in layout points, so it keeps its shape at every zoom level).
- [x] **Host-driven resize**: drag the edge of Reaper's FX window – the plugin follows, so it
      never has to extend past the bottom of the screen. Implemented in the vendored copy of
      NIH-plug (CLAP: `gui.can_resize/get_resize_hints/adjust_size/set_size`,
      VST3: `canResize/onSize/checkSizeConstraint`), see `vendor/nih_plug/VENDORED.md`.
- [x] **Language from the system locale** on first start (German systems → German,
      everything else → English); switchable at any time.

## Ideas / to discuss

- [ ] Easier fades in the timeline (originally: “pitch-shift-like” fades). Today: CC lanes
      and poly aftertouch. Possible: a fade/release time per channel in the fixture file.
- [ ] RDM – out of scope for now
