"""Builds the macOS icon art: icon-1024.png (dock icon, fed to `tauri icon`
for icon.icns) and ../tray-template@2x.png (menu bar template image).

Needs rsvg-convert and Pillow. Run from this folder: python3 build.py
"""
import random
import subprocess

from PIL import Image, ImageChops, ImageDraw, ImageFilter

S = 1024
BODY = (100, 100, 924, 924)  # apple's grid: an 824 body on a 1024 canvas
RADIUS = 185
TOP, BOTTOM = 0x1D, 0x05


def rounded_mask(scale=4):
    big = Image.new("L", (S * scale, S * scale), 0)
    ImageDraw.Draw(big).rounded_rectangle(
        [c * scale for c in BODY], RADIUS * scale, fill=255
    )
    return big.resize((S, S), Image.LANCZOS)


def body_fill():
    # a smooth vertical gradient, dithered so it doesn't band
    rnd = random.Random(7)
    img = Image.new("L", (S, S))
    px = img.load()
    for y in range(S):
        t = min(max((y - BODY[1]) / (BODY[3] - BODY[1]), 0), 1)
        v = TOP + (BOTTOM - TOP) * t
        for x in range(S):
            px[x, y] = int(v + rnd.random())
    return Image.merge("RGB", (img, img, img))


def main():
    mask = rounded_mask()

    shadow_alpha = mask.filter(ImageFilter.GaussianBlur(14)).point(lambda a: int(a * 0.4))
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    shadow.putalpha(ImageChops.offset(shadow_alpha, 0, 12))

    body = body_fill().convert("RGBA")
    body.putalpha(mask)

    edge = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle(BODY, RADIUS, outline=(255, 255, 255, 23), width=2)

    subprocess.run(["rsvg-convert", "-w", str(S), "-h", str(S), "app-icon.svg", "-o", "/tmp/mimic-mark.png"], check=True)
    mark = Image.open("/tmp/mimic-mark.png").convert("RGBA")

    out = Image.alpha_composite(shadow, body)
    out = Image.alpha_composite(out, edge)
    out = Image.alpha_composite(out, mark)
    out.save("icon-1024.png")

    subprocess.run(["rsvg-convert", "-w", "36", "-h", "36", "tray-template.svg", "-o", "../tray-template@2x.png"], check=True)


if __name__ == "__main__":
    main()
