# Railway deployment

Production runs on Railway: the Topcoat web container, one SurrealDB service,
and a `cloudflared` Tunnel connector. Cloudflare keeps DNS and CDN duties only.
The database is private Railway infrastructure and is never a Tunnel ingress.

## Services

- **benjisponge.com** — build from
  [deploy/Dockerfile](../deploy/Dockerfile), not Railpack. Railpack skips
  `topcoat asset bundle`, and the binary panics without
  `assets/manifest.toml`. Keep the service private with `PORT=8080`.
- **surrealdb** — build from
  [deploy/surrealdb.railway.toml](../deploy/surrealdb.railway.toml), which
  selects [deploy/surrealdb.Dockerfile](../deploy/surrealdb.Dockerfile). The
  image pins server `v3.2.3`, listens on private port `8000`, and stores
  RocksDB at `/home/nonroot/data.db`.
- **cloudflared** — build from
  [deploy/cloudflared.Dockerfile](../deploy/cloudflared.Dockerfile). Set
  `TUNNEL_TOKEN`; public hostnames point to
  `http://benjispongecom.railway.internal:8080`.

The database service must have exactly one replica. Its RocksDB files live on
one attached volume and cannot be shared safely by multiple service replicas.
Attach a persistent Railway volume at `/home/nonroot`, schedule Railway volume
backups, and do not expose a public domain or TCP proxy.

## Database and secrets

Set these on the `surrealdb` service before its first start:

```text
RAILWAY_RUN_UID=0
PORT=8000
SURREAL_USER=root
SURREAL_PASS=<strong generated secret>
```

Those values establish the root credentials on an empty volume. Changing the
variables later does not rotate credentials already stored in the database;
perform an intentional credential rotation instead. Railway mounts volumes as
root; `RAILWAY_RUN_UID=0` matches the derived image's `USER root` and lets
RocksDB write the attached volume. `PORT=8000` aligns Railway's `/health`
deployment check with the server's private listener.

Set all five connection variables on the web service:

```text
SURREALDB_ENDPOINT=ws://surrealdb.railway.internal:8000
SURREALDB_NAMESPACE=benjisponge
SURREALDB_DATABASE=benjisponge
SURREALDB_USERNAME=${{surrealdb.SURREAL_USER}}
SURREALDB_PASSWORD=${{surrealdb.SURREAL_PASS}}
```

The endpoint assumes the Railway service is named `surrealdb`. The two
credential references keep the web and database services aligned without
duplicating the secret.

Also set on the web service: `SPIRE_SYNC_TOKEN`, `FITNESS_SYNC_TOKEN`,
`SITE_ORIGIN=https://benjisponge.com`, and, for sign-in
([auth.md](auth.md)), `COOKIE_KEY`, `GOOGLE_OAUTH_CLIENT_ID`,
`GOOGLE_OAUTH_CLIENT_SECRET`, and `HIDDEN_PAGE_ACCESS`.

`HOST=0.0.0.0` is baked into the web image; Railway injects `PORT`. Pin it to
`8080` so the Tunnel origin stays stable.

## Clean database bootstrap

There is no migration command or migration history. The application applies
the complete `src/schema.surql` definition on its first data-backed database
connection and checks every statement result.

For a new production database:

1. Create the single-replica `surrealdb` service, credentials, and
   `/home/nonroot` volume.
2. Configure all five connection variables on the web service.
3. Deploy the web service and request a data-backed route such as
   `/api/spire/runs`; this connects and installs the schema.
4. Run `just sync-spire`, then `just sync-fitness <csv>` from the machines
   holding the source files.

This project deliberately starts clean; there is no legacy import or
compatibility step. Upgrade the pinned database image deliberately and take a
volume backup first.

`just sync-spire` discovers both games on Linux: Slay the Spire 1 below the
Steam install's `SlayTheSpire/runs` character directories, and Slay the Spire
2 below its local and Steam Cloud history directories. `--history-dir <path>`
adds another root and detects the game from each `.run` file. Run identity is
`game:id`; old database rows without a game discriminator are read as StS2,
so applying the current schema upgrades an existing production database
without a data rewrite. Deploy the game-aware web service before running the
matching sync client; the client rejects a legacy unscoped IDs response.
Slay the Spire 1 does not put aggregate profile totals in the run log and
those totals cannot recreate individual entries; when an installation has no
`runs*` directory, the sync command reports that explicitly.

## Cloudflare edge

Proxied CNAMEs for the apex, `www`, and `railway` point to
`<tunnel-id>.cfargotunnel.com`. A Redirect Rule sends `www` to the apex with
301 because planes QR codes bake the host.

Use a Cache Rule with edge TTL `respect_origin` / `bypass_by_default` so
origin `Cache-Control` wins. A later rule matching
`http.cookie contains "__Host-viewer"` must bypass cache; signed-in HTML has a
personalized shell ([auth.md](auth.md)).

Default HTML is `public, max-age=0, s-maxage=86400` from `shell`;
Spire/home/feed use `s-maxage=60`; lifting and API responses use `no-store`.
Hashed `/_topcoat/assets/*` files are immutable. `just deploy` can purge the
zone when a change must appear immediately.

## Cutover checklist

1. Confirm the database is one replica, its `/home/nonroot` volume is mounted,
   its health check passes, and backups are scheduled.
2. Confirm the database service has `RAILWAY_RUN_UID=0`, then confirm the web
   service has all five database variables plus sync and auth secrets.
3. Deploy the web app, exercise a data-backed route to bootstrap the schema,
   then sync Spire and fitness into the clean database.
4. Confirm the Railway `cloudflared` connector is healthy.
5. Confirm proxied CNAMEs for `railway`, apex, and `www` target
   `ef6f5558-8eff-4d99-a113-03df63444810.cfargotunnel.com`.
6. Confirm the origin-respecting Cache Rule, then the later
   `__Host-viewer` bypass rule, and the `www` → apex Redirect Rule.
7. Verify `https://railway.benjisponge.com`, then the apex. Remove any public
   Railway web domain so the origin stays private-only.

## Deploy

Railway's GitHub App builds `deploy/Dockerfile` and deploys the web service on
push to `main`; CI only runs `just check`. Set the database service's Railway
config-file path to `/deploy/surrealdb.railway.toml`.

```sh
just deploy
```

That command explicitly links and uploads only the web service, then purges
the Cloudflare zone. It needs a logged-in Railway CLI (or `RAILWAY_TOKEN`) and
`CLOUDFLARE_API_TOKEN`.

For Tunnel, DNS, and cache details, see
[cloudflare-deploy.md](cloudflare-deploy.md).
