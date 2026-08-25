# Cloudflare (DNS, Tunnel, CDN)

Production origin is Railway (see [railway-deploy.md](railway-deploy.md)).
Cloudflare keeps DNS, the Tunnel public hostnames, CDN cache rules, and the
public-domain redirects. `https://ben.soy` is canonical; the other two public
domains redirect to it. The TypeScript Worker + Containers under `deploy/`
are retired for production.

## Request flow

- Proxied DNS → Cloudflare CDN → Tunnel → `cloudflared` on Railway →
  topcoat at `http://benjispongecom.railway.internal:8080`.
- Cache eligibility comes from a Cache Rule; TTLs come from origin
  `Cache-Control` (`s-maxage` for edge, `max-age` for browsers).
- `/api/*` and `no-store` responses are not cached.
- `www.ben.soy` → `ben.soy` (Redirect Rule).
- `benjisponge.com`, `www.benjisponge.com`, `benmberman.com`, and
  `www.benmberman.com` → `ben.soy` (Redirect Rule, preserving path/query).
  The planes page bakes the Host header into its QR-code URL, so all public
  aliases must redirect before reaching the origin.

## Cache Rules

Cloudflare does not make extensionless HTML eligible for cache by default.
Configure these rules under **ben.soy → Caching → Cache Rules**, in this order:

1. **Origin-declared public content**
   - Expression: `(http.host eq "ben.soy" and http.request.method in {"GET" "HEAD"})`
   - Cache eligibility: **Eligible for cache**
   - Edge TTL: **Use cache-control header if present, bypass cache if not**
   - Browser TTL: **Respect origin TTL** (otherwise the zone's Browser Cache TTL
     can extend `max-age=0` and make browsers retain HTML)
2. **Viewer cookie bypass** (later, so it wins)
   - Expression: `(http.host eq "ben.soy" and http.cookie contains "__Host-viewer")`
   - Cache eligibility: **Bypass cache**

Never select an edge TTL that ignores origin cache-control. Public pages opt in
with `public` plus `s-maxage`; API, auth, admin, hidden, diary, and personalized
responses remain `no-store`. The request-side viewer bypass prevents a signed-in
visitor from receiving an anonymous cached shell, while
`src/app/response_layer.rs` prevents their personalized render from being
stored even if the Cloudflare rule drifts.

After a deploy or purge, make two anonymous requests to a public HTML path. The
first should report `CF-Cache-Status: MISS` and the second `HIT`; a request with
any `__Host-viewer` cookie must report `BYPASS` or `DYNAMIC` and
`Cache-Control: private, no-store`.

## Tunnel

Tunnel name `benjisponge`; connector runs as the Railway `cloudflared`
service with `TUNNEL_TOKEN`. Ingress hostnames:

- `ben.soy`, `www.ben.soy`, `railway.benjisponge.com` →
  `http://benjispongecom.railway.internal:8080`
- The legacy public hosts remain proxied so their Redirect Rules can run, but
  they should never reach the origin when the rules are healthy.
- `db.benjisponge.com` → `http://surrealdb.railway.internal:8000` — ONLY
  while diary direct sync is flagged on ([diary-sync.md](diary-sync.md)).
  The browser connects `wss://db.benjisponge.com` (Cloudflare proxies
  websockets; SurrealDB's own CORS is permissive) and authenticates with a
  minted record-access token, so the hostname exposes nothing without one:
  every table is `PERMISSIONS NONE` except the diary grant, and the access
  method itself only exists while `DIARY_SYNC_JWT_PUBLIC_KEY` is set.
  Remove the ingress and DNS record when the flag is off.

DNS: CNAME each public host to `<tunnel-id>.cfargotunnel.com` (proxied).
The apex and `www` records in the three public zones are needed even for
redirect-only hosts; Redirect Rules execute at the Cloudflare edge.

## Secrets / sync

`SURREALDB_ENDPOINT`, `SURREALDB_NAMESPACE`, `SURREALDB_DATABASE`,
`SURREALDB_USERNAME`, `SURREALDB_PASSWORD`, and the sync tokens live on the
Railway web service, not as Worker secrets. The database service stays on
Railway's private network and is not a Tunnel hostname — except the
flag-gated `db.` ingress above while diary direct sync is on. Spire/fitness write
paths and CLI usage are unchanged; point `just sync-spire` /
`just sync-fitness` at `https://ben.soy`.

## Historical Worker notes

The former Worker (`deploy/src/index.ts`) owned edge Cache API keys keyed
by `RELEASE_ID`, injected `s-maxage=86400` for HTML without a header, and
served `/_topcoat/assets/*` from the Workers static-asset layer. Those jobs
moved to origin headers + CDN Cache Rules + optional purge (`just deploy`),
and assets are served by the container (immutable hashed URLs).
