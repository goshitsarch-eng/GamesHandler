#!/usr/bin/env bash
#
# release-notes.sh — the GitHub Release body for a tag, on stdout.
#
#   scripts/release-notes.sh 0.8.0
#
# The curated notes already live in the AppStream metainfo <release> block —
# that is what app stores show, so the GitHub Release shows the same text
# rather than an auto-generated commit list. The XML is flattened to plain
# markdown: paragraphs stay paragraphs, <ul>/<li> become a bullet list,
# <em>/<code> become */` spans.
#
# If the version has no <release> entry the output is a one-line fallback, so
# a caller can always pass the result to `gh release create --notes-file`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

[ $# -eq 1 ] || { echo "usage: scripts/release-notes.sh VERSION" >&2; exit 2; }
VERSION="$1"

python3 - "$ROOT/data/com.goshapps.GameHandler.metainfo.xml" "$VERSION" <<'PY'
import re, sys, xml.etree.ElementTree as ET

metafile, version = sys.argv[1], sys.argv[2]
root = ET.parse(metafile).getroot()

node = None
for rel in root.findall("./releases/release"):
    if rel.get("version") == version:
        node = rel.find("description")
        break

if node is None:
    print(f"GameHandler {version}")
    sys.exit(0)

def text(el):
    out = el.text or ""
    for child in el:
        tag = child.tag
        inner = (child.text or "").strip()
        if tag in ("em", "i"):
            out += f"*{inner}*"
        elif tag == "code":
            out += f"`{inner}`"
        else:
            out += inner
        out += child.tail or ""
    return re.sub(r"\s+", " ", out).strip()

body = []
for el in node:
    if el.tag == "p":
        body.append(text(el))
    elif el.tag in ("ul", "ol"):
        body.append("\n".join(f"- {text(li)}" for li in el.findall("li")))
print("\n\n".join(b for b in body if b))
PY
