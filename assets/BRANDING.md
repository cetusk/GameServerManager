# Application artwork

The application uses these three ready-to-use files directly:

| File | Used by |
|---|---|
| `logo-dark-trimmed.png` | Native sidebar and both README files |
| `app-icon-256.png` | Native window icon |
| `app-icon.ico` | Windows executable icon (16–256 px) |

No image generation step or Python image library is required to build the app.
Slint embeds the PNGs; `winresource` embeds the ICO using the Windows SDK resource compiler.

Game-specific catalog icons are separate, under
[`apps/manager-gui/assets/game-icons`](../apps/manager-gui/assets/game-icons/SOURCES.md).
Their attribution and rights are documented there.
