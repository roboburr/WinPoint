"""Contact sheets of the Windows Update screenshots for review (GW3): the content area (hero, InfoBars, rows) of
several states side by side, labelled (shots/sheets/ holds nl-1 … en-9).

    python3 -I tests/update-ui/sheet.py LANGDIR OUT.png ID [ID …]     LANGDIR below shots/ (nl, en, …)
"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw

here = Path(__file__).resolve().parent
src, out, ids = here / "shots" / sys.argv[1], Path(sys.argv[2]), sys.argv[3:]
box = (300, 100, 1260, 760)        # the content column of a 1280 px shot: hero, InfoBars, rows
tiles = []
for i in ids:
    im = Image.open(src / f"{i}.png").convert("RGB")
    b = box if im.width >= 1260 else (0, 100, im.width, 760)
    tiles.append((i, im.crop(b)))
w = max(t.width for _, t in tiles)
cols = 2
rows = (len(tiles) + cols - 1) // cols
h = max(t.height for _, t in tiles) + 22
sheet = Image.new("RGB", (cols * w + (cols - 1) * 8, rows * h), "white")
d = ImageDraw.Draw(sheet)
for k, (i, t) in enumerate(tiles):
    x, y = (k % cols) * (w + 8), (k // cols) * h
    d.text((x + 4, y + 4), i, fill=(200, 0, 0))
    sheet.paste(t, (x, y + 22))
out.parent.mkdir(parents=True, exist_ok=True)
sheet.save(out)
print(out, sheet.size)
