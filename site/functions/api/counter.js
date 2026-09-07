/**
 * Cloudflare Pages Function — /api/counter
 *
 * GET  → returns current { visits, unique } without incrementing (for display)
 * POST → increments visits; increments unique if this IP hasn't been seen in 30 days
 *
 * KV binding: COUNTER (bound in site/wrangler.toml)
 */
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

  return Response.json({ visits, unique })
}

export async function onRequestGet({ env }) {
  if (!env.COUNTER) return Response.json({ error: 'KV not bound' }, { status: 500 })
  const visits = parseInt(await env.COUNTER.get('visits') || '0')
  const unique = parseInt(await env.COUNTER.get('unique') || '0')
  return Response.json({ visits, unique })
}
