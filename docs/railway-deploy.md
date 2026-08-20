# Railway deployment

Production runs on Railway: the Topcoat web container, one SurrealDB service,
and a `cloudflared` Tunnel connector. Cloudflare keeps DNS and CDN duties only.
The database is private Railway infrastructure and is not a Tunnel ingress —
with one deliberate, flag-gated exception: the diary direct-sync `db.`
hostname while `DIARY_SYNC_*` is set (see below and cloudflare-deploy.md).

## Services

- **ben.soy** — build from
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
# Prefer ws://. The site binary also enables protocol-http, and http://
# can help with large fitness reads, but SurrealDB's HTTP /rpc session
# has expired ~1h after connect in production (401 Unauthorized on every
# subsequent query) while the same credentials kept working over ws://.
SURREALDB_NAMESPACE=benjisponge
SURREALDB_DATABASE=benjisponge
SURREALDB_USERNAME=${{surrealdb.SURREAL_USER}}
SURREALDB_PASSWORD=${{surrealdb.SURREAL_PASS}}
```

The endpoint assumes the Railway service is named `surrealdb`. The two
credential references keep the web and database services aligned without
duplicating the secret.

Also set on the web service: `SPIRE_SYNC_TOKEN`, `FITNESS_SYNC_TOKEN`,
`PODRICK_SYNC_TOKEN` (Bearer for `GET /api/podrick/seed`, used by local
Podrick reset — [podrick.md](podrick.md)), `SITE_ORIGIN=https://ben.soy`,
and, for sign-in ([auth.md](auth.md)), `COOKIE_KEY`, `GOOGLE_OAUTH_CLIENT_ID`,
and `GOOGLE_OAUTH_CLIENT_SECRET`. Hidden-page allowlists are database rows
managed at `/admin/permissions`, not environment variables.

Diary direct sync ([diary-sync.md](diary-sync.md)) is OFF until all three of
its variables are set on the web service — set none of them until flipping
the flag deliberately:

```text
DIARY_SYNC_JWT_PUBLIC_KEY   # ES256 public PEM; bootstrap DEFINEs the access method
DIARY_SYNC_JWT_PRIVATE_KEY  # matching PKCS#8 private PEM; mints /api/diary/token
DIARY_DIRECT_SYNC_ENDPOINT  # wss://db.benjisponge.com — MUST be a ws scheme
```

Generate the pair with `openssl ecparam -genkey -name prime256v1 -noout |
openssl pkcs8 -topk8 -nocrypt` (private) and `openssl ec -pubout` (public).
Flag-on also needs the `db.` Tunnel hostname
([cloudflare-deploy.md](cloudflare-deploy.md)); unsetting the variables
removes the access method again at the next boot.

`HOST=0.0.0.0` is baked into the web image; Railway injects `PORT`. Pin it to
`8080` so the Tunnel origin stays stable.

## Clean database bootstrap

The application applies additive table/field definitions from
`src/schema.surql`, ordered site index migrations from
`src/data/schema_migrations/`, and ordered diary migrations from
`src/data/diary_migrations/` on its first data-backed connection, checking
every statement result. Site index epochs are recorded in
`site_schema_migrations`; diary epochs are recorded in
`diary_schema_migrations`. The index baseline adopts existing definitions with
`IF NOT EXISTS`, because `DEFINE INDEX OVERWRITE` rebuilds an unchanged index
on SurrealDB 3.2.3.

A Diary Schema Epoch change requires a drained handoff: keep the web service
at one replica, let Railway switch traffic off the preceding deployment, and
only then trigger the new deployment's first data-backed request. Do not scale
two web epochs against the diary store. Each migration and ledger row commit
atomically, but the ledger cannot cancel an old request that already passed
its epoch check. Ordinary deploys with no diary migration do not need this
extra constraint.

For a new production database:

1. Create the single-replica `surrealdb` service, credentials, and
   `/home/nonroot` volume.
2. Configure all five connection variables on the web service.
3. Deploy the web service and request a data-backed route such as
   `/api/spire/runs`; this connects and installs the schema.
4. Run `just sync-spire`, then `just sync-fitness <csv>` from the machines
   holding the source files.

The same migrations preserve an existing diary in place. Upgrade the pinned
database image deliberately and take a volume backup first.

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

Proxied CNAMEs for `ben.soy`, `www.ben.soy`, and `railway.benjisponge.com`
point to `<tunnel-id>.cfargotunnel.com`; the apex/`www` records in the two
legacy zones are proxied as well. Redirect Rules send `www.ben.soy` and both
legacy public domains (including `www`) to `https://ben.soy` with 301,
preserving path/query because planes QR codes bake the host.

Use a Cache Rule with edge TTL `respect_origin` / `bypass_by_default` so
origin `Cache-Control` wins. A later rule matching
`http.cookie contains "__Host-viewer"` must bypass cache; signed-in HTML has a
personalized shell ([auth.md](auth.md)).

Default HTML is `public, max-age=0, s-maxage=86400` from `shell`;
Spire/log/feed use `s-maxage=60`; lifting and API responses use `no-store`.
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
5. Confirm proxied CNAMEs for `railway.benjisponge.com`, `ben.soy`,
   `www.ben.soy`, and each legacy-zone apex/`www` target
   `ef6f5558-8eff-4d99-a113-03df63444810.cfargotunnel.com`.
6. Confirm the origin-respecting Cache Rule, then the later
   `__Host-viewer` bypass rule, and the public-alias Redirect Rules.
7. Verify `https://railway.benjisponge.com`, then `https://ben.soy`. Remove any public
   Railway web domain so the origin stays private-only.

## Deploy

Railway's GitHub App builds `deploy/Dockerfile` and deploys the web service on
push to `main`; CI only runs `just check`. Set the database service's Railway
config-file path to `/deploy/surrealdb.railway.toml`. The Dockerfile's
`wasm-builder` stage compiles the diary queue's wasm module (docs/diary-sync.md)
in parallel with the site build and ships it as `/app/wasm-dist`; if that stage
is ever dropped, the site still serves — the diary just loses its offline queue.

```sh
just deploy
```

That command explicitly links and uploads only the web service, then purges
the Cloudflare zone. It needs a logged-in Railway CLI (or `RAILWAY_TOKEN`) and
`CLOUDFLARE_API_TOKEN`.

For Tunnel, DNS, and cache details, see
[cloudflare-deploy.md](cloudflare-deploy.md).
