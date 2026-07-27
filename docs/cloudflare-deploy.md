# Cloudflare (DNS, Tunnel, CDN)

Production origin is Railway (see [railway-deploy.md](railway-deploy.md)).
This zone keeps DNS, the Cloudflare Tunnel public hostnames, CDN cache
rules, and the www→apex redirect. The TypeScript Worker + Containers under
`deploy/` are retired for production.

## Request flow

- Proxied DNS → Cloudflare CDN → Tunnel → `cloudflared` on Railway →
  topcoat at `http://benjispongecom.railway.internal:8080`.
- Cache eligibility comes from a Cache Rule; TTLs come from origin
  `Cache-Control` (`s-maxage` for edge, `max-age` for browsers).
- `/api/*` and `no-store` responses are not cached.
- www.benjisponge.com → 301 apex (Redirect Rule). Required: the planes page
  bakes the Host header into its QR-code URL.

## Tunnel

Tunnel name `benjisponge`; connector runs as the Railway `cloudflared`
service with `TUNNEL_TOKEN`. Ingress hostnames:

- `benjisponge.com`, `www.benjisponge.com`, `railway.benjisponge.com` →
  `http://benjispongecom.railway.internal:8080`

DNS: CNAME each hostname to `<tunnel-id>.cfargotunnel.com` (proxied).

## Secrets / sync

`SURREALDB_ENDPOINT`, `SURREALDB_NAMESPACE`, `SURREALDB_DATABASE`,
`SURREALDB_USERNAME`, `SURREALDB_PASSWORD`, and the sync tokens live on the
Railway web service, not as Worker secrets. The database service stays on
Railway's private network and is not a Tunnel hostname. Spire/fitness write
paths and CLI usage are unchanged; point `just sync-spire` /
`just sync-fitness` at `https://benjisponge.com`.

## Historical Worker notes

The former Worker (`deploy/src/index.ts`) owned edge Cache API keys keyed
by `RELEASE_ID`, injected `s-maxage=86400` for HTML without a header, and
served `/_topcoat/assets/*` from the Workers static-asset layer. Those jobs
moved to origin headers + CDN Cache Rules + optional purge (`just deploy`),
and assets are served by the container (immutable hashed URLs).
