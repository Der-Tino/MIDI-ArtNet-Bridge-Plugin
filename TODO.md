# TODO / Roadmap

## Window size

- [ ] **Resize grip (FabFilter style)** in the bottom-right corner of the plugin: drag to make
      the window wider/flatter while the zoom level (size preset) stays the same; all pages
      scroll to the bottom. Store the dragged size in `oberflaeche.txt` and restore it on open.
      Plus a "default size" button.
- [ ] **Screen edge / host-driven resize** (later, optional): let the user resize the plugin
      by dragging the Reaper FX window edge, so the plugin never extends past the bottom of the
      screen. Needs host→plugin GUI resizing, which NIH-plug does not implement yet
      (`ext_gui_can_resize` / `ext_gui_set_size` are TODO in the CLAP wrapper) – would mean
      patching nih-plug itself.

## Ideas

- [ ] Timeline-friendly fades (pitch-bend-like curves for fades) – revisit
- [ ] RDM is out of scope for now
