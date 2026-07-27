# Railway deployment

Production runs on Railway: topcoat container + Postgres + cloudflared
Tunnel. Cloudflare keeps DNS/CDN only (no Worker/Containers).

## Services

- **benjisponge.com** — [deploy/Dockerfile](../deploy/Dockerfile) (not
  Railpack: Railpack skips `topcoat asset bundle` and the binary panics
  without `assets/manifest.toml`). Private only; `PORT=8080`.
- **Postgres** — `POSTGRES_URL=${{Postgres.DATABASE_URL}}` on the web
  service (app reads `POSTGRES_URL`, not `DATABASE_URL`).
- **cloudflared** — [deploy/cloudflared.Dockerfile](../deploy/cloudflared.Dockerfile);
  `TUNNEL_TOKEN` from a Cloudflare Tunnel whose public hostnames point at
  `http://benjispongecom.railway.internal:8080`.

Also set on the web service: `SPIRE_SYNC_TOKEN`, `FITNESS_SYNC_TOKEN`,
`SITE_ORIGIN=https://benjisponge.com`, and — for sign-in
([auth.md](auth.md)) — `COOKIE_KEY`, `GOOGLE_OAUTH_CLIENT_ID`,
`GOOGLE_OAUTH_CLIENT_SECRET`, `HIDDEN_PAGE_ACCESS`.

`HOST=0.0.0.0` is baked into the image; Railway injects `PORT` (pin `8080`
so the tunnel origin stays stable).

## Cloudflare edge

DNS (proxied) CNAMEs apex/`www`/`railway` →
`<tunnel-id>.cfargotunnel.com`. Redirect Rule: `www` → apex 301 (planes QR
codes bake Host).

Cache Rule: Eligible for cache on the zone, edge TTL
`respect_origin` / `bypass_by_default` so origin `Cache-Control` wins.
A second rule — `http.cookie contains "__Host-viewer"` → Bypass cache,
listed after the first (later cache rules win) — keeps signed-in
visitors off the cached anonymous HTML so they actually see their
personalized shell ([auth.md](auth.md)).
Default HTML is `public, max-age=0, s-maxage=86400` from `shell`
([src/components/chrome.rs](../src/components/chrome.rs)); spire/home/feed
set `s-maxage=60`; lifting/API set `no-store`. Hashed `/_topcoat/assets/*`
are immutable from the container. Pages expire via origin `s-maxage`;
`just deploy` can purge the zone when a change must show immediately.

## Migrations

Schema in `toasty/migrations/`. Runtime image has no `migrate` binary —
apply from a machine with the repo:

```sh
POSTGRES_URL='postgresql://…' just migrate migration apply
```

Or put Railway's public `POSTGRES_URL` in `.env` and run the same. `just
dev` only migrates local Docker Postgres.

Empty DB: migrate, then `just sync-spire` / `just sync-fitness` against
`https://benjisponge.com` (tokens must match the web service).

## Cutover checklist

1. Tunnel connector healthy on Railway (`cloudflared` service Online).
2. DNS (proxied CNAMEs) for `railway`, apex, and `www` →
   `ef6f5558-8eff-4d99-a113-03df63444810.cfargotunnel.com`.
3. Cache Rules: Eligible for cache; edge TTL respect origin / bypass if no
   `Cache-Control`. Then the `__Host-viewer` cookie → Bypass rule
   ([auth.md](auth.md)).
4. Redirect Rule: `www.benjisponge.com` → `https://benjisponge.com` 301.
5. Migrate + sync (empty Postgres): `just migrate migration apply`, then
   `just sync-spire` / `just sync-fitness`.
6. Verify on `https://railway.benjisponge.com`, then apex; remove Worker
   custom domains / Containers when apex is live.
7. Optional: delete the Railway `*.up.railway.app` service domain so the
   origin stays private-only.

## Deploy

Railway's GitHub App builds `deploy/Dockerfile` and deploys the web service
on push to `main`. CI only runs `just check`. CDN pages expire via
`s-maxage`; purge manually when a deploy must show immediately:

```sh
just deploy   # optional: railway up + Cloudflare purge
```

`just deploy` needs a logged-in Railway CLI (or `RAILWAY_TOKEN`) and
`CLOUDFLARE_API_TOKEN`.

Touching Tunnel/DNS/cache rules? This doc. Old Worker notes:
[cloudflare-deploy.md](cloudflare-deploy.md).
