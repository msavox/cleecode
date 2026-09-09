/**
 * Cloudflare Pages Function — /api/counter
 *
 * GET  → returns current { visits, unique, users? } without incrementing (for display)
 * POST → increments visits; increments unique if this IP hasn't been seen in 30 days
 *
 * KV binding: COUNTER (bound in site/wrangler.toml)
 *
 * ---- users: estimated CleeCode users ----
 * Nothing about "users" is stored as a running total — it is recomputed from
 * GitHub's own release download counters, which already know who has fetched
 * a build. The formula:
 *
 *   1. List every release of the repo (paginating past 100 if there are more).
 *   2. Sort each release's assets into platform families by filename glob:
 *        linux    linux-*.tar.gz
 *        macos    macos-*.tar.gz
 *        windows  windows-*.zip
 *        deb      *.deb
 *        brew     *-src.tar.gz   (no such asset exists yet — contributes 0
 *                                 until one is published, never an error)
 *      *.sha256 checksums are never counted.
 *   3. For each family, take the MAX over all releases of that release's
 *      summed download_count for that family: someone who upgrades every
 *      version still counts once per family, someone who only ever grabbed
 *      an old build still counts at all.
 *   4. users = sum of the five family maxima.
 *
 * The GitHub call is slow, rate-limited, and pointless to repeat on every
 * page load, so the result sits in KV (key users_est) with its timestamp.
 * The entry never expires: past its 6-hour freshness window a refresh is
 * attempted, but if GitHub can't be reached the stale value is served
 * anyway — the counter may lag, it must never vanish. visits and unique
 * never depend on GitHub being reachable.
 *
 * Unauthenticated GitHub API calls are limited to 60/hour per IP, and Pages
 * Functions egress from shared Cloudflare IPs where that budget is always
 * exhausted — so a GITHUB_TOKEN secret (read-only, public repo) should be
 * set on the Pages project to get a per-token 5000/hour limit instead.
 */

const GITHUB_REPO = 'msavox/cleecode'
const USERS_CACHE_KEY = 'users_est'
const USERS_FRESH_MS = 6 * 60 * 60 * 1000 // refresh from GitHub past this age

const FAMILY_PATTERNS = {
  linux:   /linux-.*\.tar\.gz$/,
  macos:   /macos-.*\.tar\.gz$/,
  windows: /windows-.*\.zip$/,
  deb:     /\.deb$/,
  brew:    /-src\.tar\.gz$/,
}

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

function estimateUsers(releases) {
  const maxima = {}
  for (const family in FAMILY_PATTERNS) maxima[family] = 0

  for (const release of releases) {
    const sums = {}
    for (const family in FAMILY_PATTERNS) sums[family] = 0
    for (const asset of release.assets || []) {
      const name = asset.name
      if (name.endsWith('.sha256')) continue
      for (const family in FAMILY_PATTERNS) {
        if (FAMILY_PATTERNS[family].test(name)) sums[family] += asset.download_count
      }
    }
    for (const family in FAMILY_PATTERNS) {
      if (sums[family] > maxima[family]) maxima[family] = sums[family]
    }
  }

  let users = 0
  for (const family in maxima) users += maxima[family]
  return users
}

// Shared by GET and POST. Serves the cached value while it's fresh; past the
// freshness window it asks GitHub again, but a failed refresh falls back to
// the stale value rather than dropping users. Never throws — returns
// undefined only when there is no cached value at all AND GitHub can't be
// reached, so the caller can simply omit users from the response.
async function getUsersEstimate(env) {
  let stale
  const cached = await env.COUNTER.get(USERS_CACHE_KEY)
  if (cached) {
    try {
      const parsed = JSON.parse(cached)
      if (typeof parsed.users === 'number') {
        if (Date.now() - Date.parse(parsed.at || 0) < USERS_FRESH_MS) return parsed.users
        stale = parsed.users
      }
    } catch {
      // corrupt entry — fall through and recompute
    }
  }
  try {
    const users = estimateUsers(await fetchAllReleases(env))
    await env.COUNTER.put(
      USERS_CACHE_KEY,
      JSON.stringify({ users, at: new Date().toISOString() })
    )
    return users
  } catch {
    return stale
  }
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

  const users = await getUsersEstimate(env)
  const body = { visits, unique }
  if (typeof users === 'number') body.users = users
  return Response.json(body)
}

export async function onRequestGet({ env }) {
  if (!env.COUNTER) return Response.json({ error: 'KV not bound' }, { status: 500 })
  const visits = parseInt(await env.COUNTER.get('visits') || '0')
  const unique = parseInt(await env.COUNTER.get('unique') || '0')
  const users  = await getUsersEstimate(env)
  const body = { visits, unique }
  if (typeof users === 'number') body.users = users
  return Response.json(body)
}
