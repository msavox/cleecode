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
#
# The house logo is vendored beside this script as site/marunja_logo_512.png —
# copied once from marunja-suite/resources, so the site builds on a machine
# that has only this repo. The header's two typefaces (Quicksand for the
# wordmark, Open Sans for nav/text) are vendored under site/fonts/ — see
# site/fonts/OFL-NOTICE.txt for their SIL OFL provenance.

set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
docs="$here/../docs"
dist="$here/dist"
fonts="$here/fonts"
logo_src="$here/marunja_logo_512.png"

# The images the page uses: "<path under docs/>  <name under dist/assets/>".
ASSETS='
demo.gif                      demo.gif
agent.gif                     agent.gif
screenshots/preview-image.jpg preview-image.jpg
screenshots/preview-pdf.png   preview-pdf.png
screenshots/preview-md.png    preview-md.png
screenshots/octave-ide.png    octave-ide.png
screenshots/pylab-ide.png     pylab-ide.png
screenshots/debug.png         debug.png
screenshots/themes.png        themes.png
'

# The header typefaces, vendored under site/fonts/ (see OFL-NOTICE.txt there).
FONTS='
quicksand-variable.woff2
opensans-variable.woff2
'

rm -rf "$dist"
mkdir -p "$dist/assets" "$dist/fonts"

# The stylesheet ships under a name carrying a hash of its own contents, and the
# page is rewritten to point at that name. It is the only defence against a stale
# stylesheet that does not depend on somebody's dashboard: cleecode.marunja.com
# sits in a zone whose Browser Cache TTL raises any short max-age to four hours,
# so a deploy that reuses the name style.css goes on serving the old design to
# everyone who visited today — the words change, the layout does not, and the
# result looks like a deploy that failed. A new name is a new URL, and no cache
# anywhere can mistake it for the old one.
#
# Eight hex digits: enough that two stylesheets of this project will not collide,
# short enough to read in a network tab.
css_hash=$(
  if   command -v shasum   >/dev/null 2>&1; then shasum -a 256 "$here/style.css"
  elif command -v sha256sum >/dev/null 2>&1; then sha256sum    "$here/style.css"
  else cksum "$here/style.css"
  fi | cut -c1-8
)
css_name="style.$css_hash.css"

cp "$here/style.css" "$dist/$css_name"
# If the page ever spells the link differently, this substitution quietly does
# nothing — and the reference check at the end catches it, because index.html
# would then point at a style.css that dist/ does not have.
sed "s|href=\"style\.css\"|href=\"$css_name\"|" "$here/index.html" > "$dist/index.html"
echo "  page   index.html, $css_name"

# The Italian page, at /it/. Its references are absolute — from a subdirectory a
# relative "assets/…" would point at /it/assets/, which does not exist — so the
# hash substitution matches the absolute spelling, and the reference check below
# strips the leading slash before asking dist/ whether the file is there.
mkdir -p "$dist/it"
sed "s|href=\"/style\.css\"|href=\"/$css_name\"|" "$here/index.it.html" > "$dist/it/index.html"
echo "  page   it/index.html, $css_name"

# Cache policy for the deploy. Named with a leading underscore because that is
# what Cloudflare Pages looks for, and it must sit at the root of what is
# uploaded — not under assets/ — or it is served as a file instead of read as a
# rule. See the comments inside it for why the page and the stylesheet must not
# be cached long.
cp "$here/_headers" "$dist/_headers"
echo "  page   _headers"

echo "$ASSETS" | while read -r src dest; do
  [ -n "${src:-}" ] || continue
  if [ ! -f "$docs/$src" ]; then
    echo "missing asset: docs/$src" >&2
    exit 1
  fi
  cp "$docs/$src" "$dist/assets/$dest"
  echo "  asset  assets/$dest  (docs/$src)"
done

if [ ! -f "$logo_src" ]; then
  echo "missing asset: $logo_src" >&2
  exit 1
fi
cp "$logo_src" "$dist/assets/marunja_logo_512.png"
echo "  asset  assets/marunja_logo_512.png  ($logo_src)"

# The site's own icon: three sizes cut from site/icon-master.png, which is the
# rendering the icon *is* — the exports are plain downscales of it, nothing
# redrawn. They go to the dist root, where favicons are looked for: the 32 is
# the tab icon, the 180 is what iOS asks for and the hero's emblem, the 512 is
# the hi-res favicon and the social-card og:image. The master stays here as
# the source and is not published.
ICONS='
icon-32.png
icon-180.png
icon-512.png
'
echo "$ICONS" | while read -r f; do
  [ -n "${f:-}" ] || continue
  if [ ! -f "$here/$f" ]; then
    echo "missing icon: site/$f" >&2
    exit 1
  fi
  cp "$here/$f" "$dist/$f"
  echo "  icon   $f"
done

echo "$FONTS" | while read -r f; do
  [ -n "${f:-}" ] || continue
  if [ ! -f "$fonts/$f" ]; then
    echo "missing font: site/fonts/$f" >&2
    exit 1
  fi
  cp "$fonts/$f" "$dist/fonts/$f"
  echo "  font   fonts/$f"
done
cp "$fonts/OFL-NOTICE.txt" "$dist/fonts/OFL-NOTICE.txt"
echo "  font   fonts/OFL-NOTICE.txt"

# Every local src=, href= and CSS url() in the page/stylesheet must resolve
# inside dist/. Absolute URLs and in-page anchors are somebody else's problem;
# everything else is ours.
missing=0
for ref in $( { grep -oE '(src|href)="[^"]*"' "$dist/index.html" "$dist/it/index.html" \
                | sed -e 's/^[^"]*"//' -e 's/"$//'; \
                grep -oE "url\([^)]*\)" "$dist/$css_name" \
                | sed -e "s/^url(//" -e "s/)$//" -e "s/^['\"]//" -e "s/['\"]$//"; } \
             | grep -vE '^(https?:|mailto:|data:|#)' | sed 's|^/||' | sort -u); do
  # A reference ending in / names a directory page; what must exist is its index.
  if [ -f "$dist/$ref" ] || [ -f "$dist/${ref}index.html" ]; then
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
