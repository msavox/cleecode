/**
 * Cloudflare Pages Function — /api/counter
 *
 * GET  → returns current { visits, unique, downloads?, downloads_at? } without
 *        incrementing (for display)
 * POST → increments visits; increments unique if this IP hasn't been seen in 30 days
 *
 * KV binding: COUNTER (bound in site/wrangler.toml)
 *
 * ---- downloads: release downloads ----
 * Nothing about "downloads" is stored as a running total — it is read from
 * GitHub's own release download counters, which already know how many times
 * each build has been fetched. The sum is every asset of every release, minus
 * the *.sha256 checksums, which are not the software.
 *
 * This replaced an earlier "estimated users" figure that took, per platform
 * family, the maximum over releases of that release's downloads. That number
 * could not keep rising: with a release every few days the downloads spread
 * across versions, so no new release ever beat the historical peak of its
 * family and the figure sat still even when the refresh was working. A plain
 * sum has no such ceiling — every download moves it, which is the only way a
 * frozen counter is distinguishable from a quiet week.
 *
 * The GitHub call is slow, rate-limited, and pointless to repeat on every
 * page load, so the result sits in KV (key downloads_total) with its
 * timestamp. The entry never expires: past its 6-hour freshness window a
 * refresh is attempted, but if GitHub can't be reached the stale value is
 * served anyway — the counter may lag, it must never vanish. Because a
 * permanently failing refresh is otherwise invisible (it just looks like a
 * number that stopped moving), the response carries downloads_at: the instant
 * the served figure was actually read from GitHub. Compare it with now to
 * tell a stalled refresh from a quiet week. visits and unique never depend on
 * GitHub being reachable.
 *
 * Unauthenticated GitHub API calls are limited to 60/hour per IP, and Pages
 * Functions egress from shared Cloudflare IPs where that budget is always
 * exhausted — so a GITHUB_TOKEN secret (read-only, public repo) must be set on
 * the Pages project to get a per-token 5000/hour limit instead. Without it the
 * refresh fails every time and the figure freezes at whatever was last cached.
 */

const GITHUB_REPO = 'msavox/cleecode'
const DOWNLOADS_CACHE_KEY = 'downloads_total'
const DOWNLOADS_FRESH_MS = 6 * 60 * 60 * 1000 // refresh from GitHub past this age

async function fetchAllReleases(env) {
  const headers = { 'User-Agent': 'cleecode-site' } // GitHub's API 4xxs any request without one
  if (env.GITHUB_TOKEN) headers['Authorization'] = 'Bearer ' + env.GITHUB_TOKEN
  const releases = []
  for (let page = 1; ; page++) {
    const res = await fetch(
      `https://api.github.com/repos/${GITHUB_REPO}/releases?per_page=100&page=${page}`,
      { headers }
    )
    if (!res.ok) throw new Error('github releases fetch failed: ' + res.status)
    const batch = await res.json()
    releases.push(...batch)
    if (batch.length < 100) break
  }
  return releases
}

// Every asset of every release, except the checksums beside them.
function countDownloads(releases) {
  let total = 0
  for (const release of releases) {
    for (const asset of release.assets || []) {
      if (asset.name.endsWith('.sha256')) continue
      total += asset.download_count
    }
  }
  return total
}

// Shared by GET and POST. Serves the cached value while it's fresh; past the
// freshness window it asks GitHub again, but a failed refresh falls back to
// the stale value rather than dropping the figure. Never throws — returns
// {} only when there is no cached value at all AND GitHub can't be reached,
// so the caller can simply omit downloads from the response. The returned
// `at` is when the figure was read from GitHub, not when it was served.
async function getDownloads(env) {
  let stale
  const cached = await env.COUNTER.get(DOWNLOADS_CACHE_KEY)
  if (cached) {
    try {
      const parsed = JSON.parse(cached)
      if (typeof parsed.downloads === 'number') {
        if (Date.now() - Date.parse(parsed.at || '') < DOWNLOADS_FRESH_MS) {
          return { downloads: parsed.downloads, at: parsed.at }
        }
        stale = { downloads: parsed.downloads, at: parsed.at }
      }
    } catch {
      // corrupt entry — fall through and recompute
    }
  }
  try {
    const downloads = countDownloads(await fetchAllReleases(env))
    const at = new Date().toISOString()
    await env.COUNTER.put(DOWNLOADS_CACHE_KEY, JSON.stringify({ downloads, at }))
    return { downloads, at }
  } catch (err) {
    // Logged, not swallowed: without this a token that stopped working shows up
    // only as a figure that quietly stopped moving. Visible in `wrangler pages
    // deployment tail` and in the dashboard's live logs.
    console.error('downloads refresh failed:', err && err.message)
    return stale || {}
  }
}

function body(visits, unique, downloads) {
  const out = { visits, unique }
  if (typeof downloads.downloads === 'number') {
    out.downloads = downloads.downloads
    if (downloads.at) out.downloads_at = downloads.at
  }
  return out
}

export async function onRequestPost({ request, env }) {
  if (!env.COUNTER) return Response.json({ error: 'KV not bound' }, { status: 500 })

  // Total visits
  const visits = parseInt(await env.COUNTER.get('visits') || '0') + 1
  await env.COUNTER.put('visits', String(visits))

  // Unique visitors — hash the IP so we never store PII
  const ip = request.headers.get('CF-Connecting-IP') ||
             request.headers.get('X-Forwarded-For')  ||
             'unknown'
  const raw    = new TextEncoder().encode(ip + ':cleecode-v1')
  const digest = await crypto.subtle.digest('SHA-256', raw)
  const key    = 'u:' + [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2,'0')).join('').slice(0, 20)

  let unique = parseInt(await env.COUNTER.get('unique') || '0')
  const seen = await env.COUNTER.get(key)
  if (!seen) {
    unique++
    await env.COUNTER.put('unique', String(unique))
    await env.COUNTER.put(key, '1', { expirationTtl: 86400 * 30 })   // 30-day window
  }

  return Response.json(body(visits, unique, await getDownloads(env)))
}

export async function onRequestGet({ env }) {
  if (!env.COUNTER) return Response.json({ error: 'KV not bound' }, { status: 500 })
  const visits = parseInt(await env.COUNTER.get('visits') || '0')
  const unique = parseInt(await env.COUNTER.get('unique') || '0')
  return Response.json(body(visits, unique, await getDownloads(env)))
}
