# Application branding

The original PNGs are the source artwork for the application.
Keep these originals unchanged.

- `logo-dark-transparent.png`: logo for the dark interface; used in the current
  native sidebar via the trimmed derivative.
- `logo-light-transparent.png`: reserved for a light background; not loaded by
  the current interface.
- `icon-transparent.png`: source application icon (original canvas preserved).
- `generated/logo-dark-trimmed.png`: dark logo with transparent outer margins
  trimmed; used by the native interface.
- `generated/app-icon-256.png`: tightly framed RGBA icon for the Slint window
  and application previews.
- `generated/app-icon.ico`: Windows executable icon, with 16, 24, 32, 48, 64,
  128 and 256 pixel images. Transparency remains; surplus canvas spacing is removed.

Derivatives crop visible alpha bounds (alpha > 8), add 2.5% padding on each
side, and center the icon on a square canvas. Original PNGs remain unchanged.

Regenerate the three derived files with Python 3 and Pillow:

```sh
python -m pip install Pillow
python tools/generate-brand-icons.py
```

The generated files are checked in, so normal Rust builds do not need Python or
Pillow. Slint embeds the logo and window icon. `winresource` embeds the ICO in
Windows builds using the Windows SDK resource compiler. Linux checks of the
Windows target use `llvm-rc`; these checks do not verify Windows Explorer or
Windows taskbar rendering.
