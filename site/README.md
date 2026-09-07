# site — the CleeCode landing page

The static one-pager for `cleecode.marunja.com`: hand-written HTML and CSS, no framework, no CDN,
no JavaScript. Sources are `index.html` and `style.css`; the pictures stay in `docs/` and are
copied in at build time, so the site reuses them rather than duplicating them. `_headers` carries
the cache policy Cloudflare Pages reads — the page and the stylesheet keep their names across
deploys, so they are served must-revalidate rather than cached for hours.

Build with `./build.sh` (or `sh site/build.sh` from the repo root). It assembles `site/dist/` —
page, stylesheet and `dist/assets/` — and fails if the page points at a file that is not there.
`dist/` is disposable output and gitignored; the asset list lives in `build.sh` and nowhere else.

Deploy: `wrangler pages deploy dist --project-name cleecode --branch main --cwd site`
(or `cd site && wrangler pages deploy dist --project-name cleecode --branch main` — same
thing). The `--branch main` is not decoration: the Pages project's production branch is
`main`, and a deploy without it lands as a *preview* named after the git branch
(`master`), which the custom domain never serves — it looks like a deploy that changed
nothing. The `--cwd site` matters too: Wrangler discovers `functions/` and
`wrangler.toml` (which carries the COUNTER KV binding) relative to its working
directory, not relative to the uploaded directory — run it from the repo root with
`site/dist` as the argument instead and the deploy silently ships static files only,
with `/api/counter` left unbound. Check the deploy output for `Uploading Functions
bundle`; if that line is missing, the Function was not picked up.
