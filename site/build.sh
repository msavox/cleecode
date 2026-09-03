#!/bin/sh
# Assemble the CleeCode landing site into site/dist/.
#
# Runs from anywhere: the repo root, site/, or an absolute path. Nothing is ever
# copied into site/ itself — dist/ is disposable output and is gitignored.
#
#   sh site/build.sh          # from the repo root
#   ./build.sh                # from inside site/
#
# This file is the single place the asset list lives. Add a picture to the page,
# add its line to ASSETS below; the check at the end refuses to finish if the
# page points at something dist/ does not have.

set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
docs="$here/../docs"
dist="$here/dist"

# The images the page uses: "<path under docs/>  <name under dist/assets/>".
ASSETS='
demo.gif                     demo.gif
screenshots/octave-ide.png   octave-ide.png
screenshots/pylab-ide.png    pylab-ide.png
screenshots/themes.png       themes.png
'

rm -rf "$dist"
mkdir -p "$dist/assets"

cp "$here/index.html" "$dist/index.html"
cp "$here/style.css"  "$dist/style.css"
echo "  page   index.html, style.css"

echo "$ASSETS" | while read -r src dest; do
  [ -n "${src:-}" ] || continue
  if [ ! -f "$docs/$src" ]; then
    echo "missing asset: docs/$src" >&2
    exit 1
  fi
  cp "$docs/$src" "$dist/assets/$dest"
  echo "  asset  assets/$dest  (docs/$src)"
done

# Every local src= and href= in the page must resolve inside dist/. Absolute
# URLs and in-page anchors are somebody else's problem; everything else is ours.
missing=0
for ref in $(grep -oE '(src|href)="[^"]*"' "$dist/index.html" \
             | sed -e 's/^[^"]*"//' -e 's/"$//' \
             | grep -vE '^(https?:|mailto:|#)' | sort -u); do
  if [ -f "$dist/$ref" ]; then
    echo "  ok     $ref"
  else
    echo "  BROKEN $ref" >&2
    missing=$((missing + 1))
  fi
done

if [ "$missing" -ne 0 ]; then
  echo "$missing referenced file(s) not in dist/" >&2
  exit 1
fi

echo "built $dist"
