#!/usr/bin/env python3
"""imgdiff.py BEFORE.png AFTER.png — report how many pixels moved.

Why this exists rather than `cmp` or a hash: two PNG captures of an identical
frame are usually *not* byte-identical, because the encoder's filter and
compression choices depend on history and the two files can differ in length
while showing the same pixels. Comparing the encoded bytes therefore reports a
difference where there is none, and — worse — the len() guard that comparison
needs turns "the images have different dimensions" into a third outcome a
caller has to remember to handle. Decoding to pixels and counting the ones that
changed answers the question the walk is actually asking: did the app react?

Exit status is 0 when the images are the same size (whatever the pixel count)
and 1 when they are not, so a caller can distinguish "nothing happened" from
"the capture is not comparable".

    changed pixels: 459 of 1024000 (0.04%)
    bounding box: x 640..1023  y 27..60
"""

import sys

import gi

gi.require_version("GdkPixbuf", "2.0")
from gi.repository import GdkPixbuf  # noqa: E402


def load(path):
    pb = GdkPixbuf.Pixbuf.new_from_file(path)
    return pb, pb.get_width(), pb.get_height(), pb.get_n_channels(), pb.get_rowstride(), pb.get_pixels()


def main(argv):
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    _, w1, h1, n1, rs1, px1 = load(argv[1])
    _, w2, h2, n2, rs2, px2 = load(argv[2])
    if (w1, h1, n1) != (w2, h2, n2):
        print(f"not comparable: {w1}x{h1}x{n1} vs {w2}x{h2}x{n2}")
        return 1

    changed = 0
    x0 = y0 = 1 << 30
    x1 = y1 = -1
    for y in range(h1):
        o = y * rs1
        for x in range(w1):
            i = o + x * n1
            if px1[i:i + n1] != px2[i:i + n1]:
                changed += 1
                if x < x0:
                    x0 = x
                if x > x1:
                    x1 = x
                if y < y0:
                    y0 = y
                if y > y1:
                    y1 = y

    total = w1 * h1
    print(f"changed pixels: {changed} of {total} ({100.0 * changed / total:.2f}%)")
    if changed:
        print(f"bounding box: x {x0}..{x1}  y {y0}..{y1}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
