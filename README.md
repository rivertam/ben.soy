# Ben's Site

## Setup

The project uses [proto](https://moonrepo.dev/proto) to pin development tools.
Install the configured toolchain and Git hooks, then start the development server:

```sh
proto use
just install-hooks
just dev
```

The site is available at <http://127.0.0.1:3000>. To use another port:

```sh
just dev 4610
```

`just dev` also starts a persistent local SurrealDB container on port 5800;
the application bootstraps its schema on connection. Seed fitness data
separately with `just reset-fitness-local [csv]` (default
`/home/benji/Downloads/WorkoutData.csv`). See `docs/fitness.md`.

If `.env.dev` configures the Discord bot, `just dev` runs it too, so lift
announcements and Pants Off ingestion can be exercised in test channels;
`just dev --no-podrick` skips it. See `docs/podrick.md`.

## Commands

Run `just` or `just --list` to see the available commands.

```sh
just build
just release
just check
```

## Deploy

- Railway origin: `just deploy` — see `railway.toml` and
  `docs/railway-deploy.md`
- Cloudflare DNS, Tunnel, and CDN: see `docs/cloudflare-deploy.md`
