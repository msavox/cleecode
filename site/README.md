# site — the CleeCode landing page

The static one-pager for `cleecode.marunja.com`: hand-written HTML and CSS, no framework, no CDN,
no JavaScript. Sources are `index.html` and `style.css`; the pictures stay in `docs/` and are
copied in at build time, so the site reuses them rather than duplicating them. `_headers` carries
the cache policy Cloudflare Pages reads — the page and the stylesheet keep their names across
deploys, so they are served must-revalidate rather than cached for hours.

Build with `./build.sh` (or `sh site/build.sh` from the repo root). It assembles `site/dist/` —
page, stylesheet and `dist/assets/` — and fails if the page points at a file that is not there.
`dist/` is disposable output and gitignored; the asset list lives in `build.sh` and nowhere else.

Deploy: `wrangler pages deploy site/dist --project-name cleecode --branch main`. The
`--branch main` is not decoration: the Pages project's production branch is `main`, and a
deploy without it lands as a *preview* named after the git branch (`master`), which the
custom domain never serves — it looks like a deploy that changed nothing.
