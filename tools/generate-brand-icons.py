"""Create tightly framed display artwork; preserve the user-supplied originals.

Requires Pillow. Run: python tools/generate-brand-icons.py
"""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parents[1]
output = root / "assets" / "generated"
output.mkdir(parents=True, exist_ok=True)


def trimmed(name, square=False):
    with Image.open(root / "assets" / name) as original:
        source = original.convert("RGBA")
        # Ignore near-invisible export residue when measuring visible artwork.
        bounds = source.getchannel("A").point(lambda a: 255 if a > 8 else 0).getbbox()
        if bounds is None:
            raise ValueError(f"Empty artwork: {name}")
        crop = source.crop(bounds)
    pad = max(2, round(max(crop.size) * 0.025))
    w, h = crop.size
    if square:
        w = h = max(w, h)
    canvas = Image.new("RGBA", (w + 2 * pad, h + 2 * pad))
    canvas.paste(crop, ((canvas.width - crop.width) // 2, (canvas.height - crop.height) // 2))
    return canvas


icon = trimmed("icon-transparent.png", square=True)
icon.resize((256, 256), Image.Resampling.LANCZOS).save(output / "app-icon-256.png")
icon.save(output / "app-icon.ico", format="ICO",
          sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
trimmed("logo-dark-transparent.png").save(output / "logo-dark-trimmed.png")
