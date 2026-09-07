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
 * page load, so the result sits in KV for 6 hours (key users_est). A visit
 * inside that window reads the cache; GitHub is only asked when it expires,
 * and if that ask fails, users is simply left out of the response — visits
 * and unique must never depend on GitHub being reachable.
 */

const GITHUB_REPO = 'msavox/cleecode'
const USERS_CACHE_KEY = 'users_est'
const USERS_CACHE_TTL = 6 * 60 * 60 // seconds

const FAMILY_PATTERNS = {
  linux:   /linux-.*\.tar\.gz$/,
  macos:   /macos-.*\.tar\.gz$/,
  windows: /windows-.*\.zip$/,
  deb:     /\.deb$/,
  brew:    /-src\.tar\.gz$/,
}

async function fetchAllReleases() {
  const releases = []
  for (let page = 1; ; page++) {
    const res = await fetch(
      `https://api.github.com/repos/${GITHUB_REPO}/releases?per_page=100&page=${page}`,
      { headers: { 'User-Agent': 'cleecode-site' } } // GitHub's API 4xxs any request without one
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

// Shared by GET and POST. Reads the live cache if KV still holds it;
// otherwise recomputes from GitHub and refreshes the cache. Never throws —
// returns undefined when there is no live cache and GitHub can't be reached,
// so the caller can simply omit users from the response.
async function getUsersEstimate(env) {
  const cached = await env.COUNTER.get(USERS_CACHE_KEY)
  if (cached) {
    try {
      const parsed = JSON.parse(cached)
      if (typeof parsed.users === 'number') return parsed.users
    } catch {
      // corrupt entry — fall through and recompute
    }
  }
  try {
    const users = estimateUsers(await fetchAllReleases())
    await env.COUNTER.put(
      USERS_CACHE_KEY,
      JSON.stringify({ users, at: new Date().toISOString() }),
      { expirationTtl: USERS_CACHE_TTL }
    )
    return users
  } catch {
    return undefined
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
