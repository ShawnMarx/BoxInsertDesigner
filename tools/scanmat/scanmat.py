#!/usr/bin/env python3
"""Print-ready scan mats: a coloured field to key out, ArUco markers to straighten the photo.

    python3 -I tools/scanmat/scanmat.py            # writes web/mats/*.pdf and web/mats/mats.json

Each mat is a US Letter page, landscape. Six ArUco markers (4x4_50 dictionary) sit in the white
border, four in the corners and one at the middle of each long edge; each mat colour has its own
marker IDs, so a photo says which mat it is on. The field is the coloured area pieces are laid
on; mm ticks run around it, and a 100 mm bar in the left border checks the page printed at 100%.

`mats.json` is the geometry the app reads: mm, origin at the page's top-left, x right, y down.
Standard library only, so it runs anywhere; the PDF is written by hand (Helvetica, not embedded).
"""

import json
import os

# ArUco DICT_4X4_50, IDs 0-17: the 4x4 data cells inside the one-cell black border, rows top to
# bottom, 1 = white. Taken from OpenCV's dictionary (cv2.aruco.generateImageMarker).
ARUCO_4X4 = {
    0: ["1011", "0101", "0011", "0010"], 1: ["0000", "1111", "1001", "1010"],
    2: ["0011", "0011", "0010", "1101"], 3: ["1001", "1001", "0100", "0110"],
    4: ["0101", "0100", "1001", "1110"], 5: ["0111", "1001", "1100", "1101"],
    6: ["1001", "1110", "0010", "1110"], 7: ["1100", "0100", "1111", "0010"],
    8: ["1111", "1110", "1101", "1010"], 9: ["1100", "1111", "0101", "0110"],
    10: ["1111", "1001", "1001", "0001"], 11: ["0001", "0001", "1010", "0111"],
    12: ["0000", "1110", "1011", "0111"], 13: ["0010", "1010", "0000", "1111"],
    14: ["0010", "0100", "1011", "0001"], 15: ["0010", "0110", "0011", "1110"],
    16: ["0100", "0110", "0110", "0101"], 17: ["0110", "0110", "0000", "0000"],
}

VERSION = 1
PAGE_W, PAGE_H = 279.4, 215.9          # US Letter, landscape
MARKER = 20.0                          # marker side, black border included
INSET = 16.5                           # marker centre from the page edges
RULER = 9.0                            # depth of the ruler strip on each side of the field
FIELD = 36.0                           # field inset from the page edges

# Two colours far apart, so a piece close to one keys out cleanly on the other. White has no
# field colour: it is for checking the print's size and the rulers, not for keying.
MATS = [
    {"key": "teal", "name": "Teal", "rgb": (0.17, 0.55, 0.49), "ids": [0, 1, 2, 3, 4, 5],
     "use": "The default. Use Magenta for green, teal or blue pieces."},
    {"key": "magenta", "name": "Magenta", "rgb": (0.80, 0.20, 0.55), "ids": [6, 7, 8, 9, 10, 11],
     "use": "For green, teal or blue pieces. Use Teal for red, pink or purple ones."},
    {"key": "white", "name": "White", "rgb": None, "ids": [12, 13, 14, 15, 16, 17],
     "use": "For checking the print's size against a ruler. No field colour, so not for scans."},
]

PT = 72 / 25.4


def marker_centres():
    """Top-left, top-middle, top-right, bottom-left, bottom-middle, bottom-right."""
    xs = [INSET, PAGE_W / 2, PAGE_W - INSET]
    return [(x, INSET) for x in xs] + [(x, PAGE_H - INSET) for x in xs]


class Page:
    """A minimal PDF content stream in mm, y down."""

    def __init__(self):
        self.ops = []

    def fill(self, rgb):
        self.ops.append("%.3f %.3f %.3f rg" % rgb)

    def rect(self, x, y, w, h):
        self.ops.append("%.3f %.3f %.3f %.3f re f" % (x * PT, (PAGE_H - y - h) * PT, w * PT, h * PT))

    def text(self, x, y, size, s, rotate=False):
        s = s.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
        m = "0 1 -1 0" if rotate else "1 0 0 1"
        self.ops.append("BT /F1 %.1f Tf %s %.3f %.3f Tm (%s) Tj ET" % (size, m, x * PT, (PAGE_H - y) * PT, s))

    def stream(self):
        return "\n".join(self.ops).encode("cp1252")


def draw_marker(p, cx, cy, mid):
    u = MARKER / 6
    x0, y0 = cx - MARKER / 2, cy - MARKER / 2
    p.fill((0, 0, 0))
    p.rect(x0, y0, MARKER, MARKER)
    p.fill((1, 1, 1))
    for r, row in enumerate(ARUCO_4X4[mid]):
        for c, bit in enumerate(row):
            if bit == "1":
                p.rect(x0 + (c + 1) * u, y0 + (r + 1) * u, u, u)


def draw_mat(mat):
    p = Page()
    fx, fy, fw, fh = FIELD, FIELD, PAGE_W - 2 * FIELD, PAGE_H - 2 * FIELD
    if mat["rgb"]:
        p.fill(mat["rgb"])
        p.rect(fx, fy, fw, fh)

    # A ruler on each side of the field: a light strip with mm ticks from the field's edge
    # (1 mm, longer every 5, longest every 10) and figures every 10 mm, counted from the
    # field's top-left corner, so 0 is the field's corner on every side.
    g, k = (0.92, 0.92, 0.90), 0.25
    side = RULER + 1.5  # the side rulers are deeper: their figures run across them
    for x, y, w, h in [(fx, fy - RULER, fw, RULER), (fx, fy + fh, fw, RULER),
                       (fx - side, fy, side, fh), (fx + fw, fy, side, fh)]:
        p.fill(g)
        p.rect(x, y, w, h)
        p.fill((0.1, 0.1, 0.1))
        p.rect(x, y, w, k); p.rect(x, y + h - k, w, k); p.rect(x, y, k, h); p.rect(x + w - k, y, k, h)

    def tick(i):
        return (5.0, 0.3) if i % 10 == 0 else (3.2, 0.25) if i % 5 == 0 else (2.0, 0.2)

    p.fill((0.05, 0.05, 0.05))
    for i in range(int(fw) + 1):
        L, t = tick(i)
        p.rect(fx + i - t / 2, fy - L, t, L)
        p.rect(fx + i - t / 2, fy + fh, t, L)
    for i in range(int(fh) + 1):
        L, t = tick(i)
        p.rect(fx - L, fy + i - t / 2, L, t)
        p.rect(fx + fw, fy + i - t / 2, L, t)

    # Helvetica digits are 0.556 em wide, so the figures can be centred on their ticks.
    fs = 5.2
    digit = 0.556 * fs / PT
    for i in range(0, int(fw) + 1, 10):
        s, x = str(i), fx + i - len(str(i)) * digit / 2
        if i == 0:
            x = fx + 0.7  # inside the strip, beside the corner
        p.text(x, fy - 5.8, fs, s)
        p.text(x, fy + fh + 7.2, fs, s)
    for j in range(0, int(fh) + 1, 10):
        s, y = str(j), fy + j + (2.4 if j == 0 else 0.65)
        p.text(fx - 5.7 - len(s) * digit, y, fs, s)
        p.text(fx + fw + 5.7, y, fs, s)

    for (cx, cy), mid in zip(marker_centres(), mat["ids"]):
        draw_marker(p, cx, cy, mid)

    # The 100 mm check bar, left border.
    bar_y0 = PAGE_H / 2 - 50
    p.fill((0, 0, 0))
    p.rect(19, bar_y0, 2.2, 100)
    p.fill((0.25, 0.25, 0.25))
    p.text(15.5, bar_y0 + 100, 6.5, "This bar is 100 mm. If not, reprint at 100% (Actual size).", rotate=True)

    p.fill((0.15, 0.15, 0.15))
    p.text(36, 15.5, 9, "BoxInsertDesigner scan mat \xb7 %s \xb7 Letter \xb7 v%d" % (mat["name"], VERSION))
    p.text(36, 22, 6.5, "Pieces flat on the field. Shoot straight down, all six markers in frame.")
    p.text(PAGE_W / 2 + 14, 15.5, 6.5, "Field %.1f \xd7 %.1f mm \xb7 ticks every 1 mm" % (fw, fh))
    p.text(PAGE_W / 2 + 14, 22, 6.5, "ArUco 4\xd74 markers, IDs %d\u2013%d" % (mat["ids"][0], mat["ids"][-1]))
    p.text(36, PAGE_H - 15, 6.5, mat["name"] + " mat: " + mat["use"])
    return p.stream()


def pdf(content):
    objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        ("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 %.2f %.2f] /Contents 4 0 R "
         "/Resources << /Font << /F1 5 0 R >> >> >>" % (PAGE_W * PT, PAGE_H * PT)).encode(),
        b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    ]
    out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for i, o in enumerate(objs, 1):
        offsets.append(len(out))
        out += b"%d 0 obj\n" % i + o + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
    out += b"".join(b"%010d 00000 n \n" % off for off in offsets)
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, xref)
    return bytes(out)


def main():
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    outdir = os.path.join(root, "web", "mats")
    os.makedirs(outdir, exist_ok=True)
    spec = {"version": VERSION, "units": "mm", "origin": "page top-left, y down",
            "dictionary": "aruco_4x4_50", "paper": "letter", "page": [PAGE_W, PAGE_H], "mats": []}
    for mat in MATS:
        file = "scan-mat-%s-letter.pdf" % mat["key"]
        with open(os.path.join(outdir, file), "wb") as f:
            f.write(pdf(draw_mat(mat)))
        spec["mats"].append({
            "key": mat["key"], "name": mat["name"], "file": file, "use": mat["use"],
            "printed_rgb": [round(c * 255) for c in mat["rgb"]] if mat["rgb"] else None,
            "field": [FIELD, FIELD, round(PAGE_W - 2 * FIELD, 2), round(PAGE_H - 2 * FIELD, 2)],
            "markers": [{"id": mid, "centre": [round(x, 2), round(y, 2)], "size": MARKER}
                        for (x, y), mid in zip(marker_centres(), mat["ids"])],
        })
        print("wrote", os.path.join("web", "mats", file))
    with open(os.path.join(outdir, "mats.json"), "w") as f:
        json.dump(spec, f, indent=2)
        f.write("\n")
    print("wrote web/mats/mats.json")


if __name__ == "__main__":
    main()
