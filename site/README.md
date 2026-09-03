# site — the CleeCode landing page

The static one-pager for `cleecode.marunja.com`: hand-written HTML and CSS, no framework, no CDN,
no JavaScript. Sources are `index.html` and `style.css`; the pictures stay in `docs/` and are
copied in at build time, so the site reuses them rather than duplicating them.

Build with `./build.sh` (or `sh site/build.sh` from the repo root). It assembles `site/dist/` —
page, stylesheet and `dist/assets/` — and fails if the page points at a file that is not there.
`dist/` is disposable output and gitignored; the asset list lives in `build.sh` and nowhere else.

Deploy (documented, not run from here): `wrangler pages deploy site/dist --project-name cleecode`.
