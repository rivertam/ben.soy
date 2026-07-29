# Podrick

A Discord bot for a server I'm in, deployed from this repo. Read this before
changing anything under `src/app/interests/podrick/`.

- **Job 1 (built):** announce a lift in a channel when one is published here.
- **Job 2 (not built):** manage database records in response to posts in a
  second channel, seeded once from that channel's history.

The public page is `/podrick`; the bot is the `podrick` binary.

## Why there is no gateway and no bot framework

Both jobs are REST shaped. Announcing is one `POST /channels/{id}/messages`.
Reading a channel — live *and* when seeding from history — is `GET
/channels/{id}/messages` walked by snowflake. A gateway connection would buy
sub-second latency in exchange for a large dependency tree, a
heartbeat/resume lifecycle to babysit, and a privileged-intent approval; it
would also split the backfill and the live tail into two code paths that can
drift. `discord.rs` is a few hundred lines over the `reqwest` this repo
already depends on.

Intents gate *gateway* events, not these endpoints. The bot needs channel
permissions — View Channel, Send Messages, Read Message History — not
privileged intents.

## Setting up the Discord side

1. https://discord.com/developers/applications → **New Application**, name it
   Podrick.
2. **Bot** → **Reset Token** → copy it. This is `DISCORD_BOT_TOKEN` and is
   shown once. Leave every Privileged Gateway Intent off.
3. **OAuth2 → URL Generator** → scope `bot`, permissions **View Channel**,
   **Send Messages**, **Read Message History**. Open the generated URL and
   install it to the server.
   - Installing requires **Manage Server** on that guild. If you do not have
     it, the owner must open the URL; there is no way around this.
4. In Discord, **Settings → Advanced → Developer Mode** on, then right-click
   the announcement channel → **Copy Channel ID**. That is
   `PODRICK_LIFT_CHANNEL_ID`.
5. Confirm the bot can actually see the channel. A private channel needs the
   bot's role added to it explicitly — installing to the guild is not enough,
   and this is the usual cause of a 403.

## Running it

```sh
just podrick once --dry-run          # render to stdout, post nothing
just podrick once                    # one pass
just podrick run --interval 30       # poll forever
just podrick --help
```

`--dry-run` is read-only: it reads the database and the site API, renders each
message to stdout, and writes nothing — no watermark, no claims, no attempt
counters. It needs no token, so message copy can be worked on before the
Discord application exists. That read-only guarantee is deliberate, because
seeding the watermark is a once-ever decision (see Invariants) and a "just
looking" run against production must not make it.

The channel id comes from `PODRICK_LIFT_CHANNEL_ID` and the token from
`--token`, else `$DISCORD_BOT_TOKEN`, else
`~/.config/benjisponge/podrick.token` — the same directory and precedence the
Spire and fitness sync clients use. The five `SURREALDB_*` connection variables
are required as usual.

`just podrick` defaults `--api` to **production** and inherits whatever
`SURREALDB_*` the shell has. It does not read `.env.dev`.

### Locally

`just dev` runs Podrick beside the site, so a lift pasted into the local
upload dialog is announced without remembering to start a second terminal. It
polls every 10 seconds locally rather than the deployed 60, and it stops when
the dev server does.

```sh
just dev                             # site + podrick
just dev --no-podrick                # site only, to drive the bot by hand
just dev --podrick-reset             # clear the local watermark, then start
just podrick-local once --dry-run    # one read-only pass, beside --no-podrick
```

It starts only when `.env.dev` names a channel *and* a token resolves. With no
channel configured `just dev` behaves exactly as it always has and says
nothing about Podrick; with a channel but no token it says so once and starts
the site anyway.

`scripts/podrick-local.sh` sources `.env.dev` for `PODRICK_LIFT_CHANNEL_ID`
(and `DISCORD_BOT_TOKEN`, if you keep it there), then pins the database and
`--api` to the local stack *after* sourcing — the same ordering, for the same
reason, as `scripts/dev.sh`: a stray production endpoint in `.env.dev` must
not be able to point the bot at the real database. `dev.sh` waits for the site
to answer before starting it, and it refuses to start on its own when no site
answers, because otherwise a cold build looks like a run of announce failures.

**The watermark is why a lift you already have does not announce.** The first
run seeds it at the newest workout that already exists, so anything pasted
after Podrick starts is announced and anything already there never is —
including, exactly, the workout it seeded from, since eligibility is *strictly*
newer. The seeding log reports UTC and `/lifting` shows Eastern, so the lift
sitting on the boundary does not look like the watermark unless you convert. To
re-announce a lift you already have locally: delete it (the delete control on
`/lifting`, or `just delete-lift`), then `just dev --podrick-reset`, then
paste it again. `--podrick-reset` clears the local `podrick_meta` and
`podrick_announcements` rows; it exists only on the dev scripts and
deliberately not on the binary, because production must not grow a backfill
switch (see Invariants).

Run `just podrick-local` by hand only alongside `just dev --no-podrick`, or
two bots poll the same database. They will not double-post — the claim is
create-only — but the logs interleave, and a `--reset` under a running bot
only makes that bot reseed.

The permalink in a locally announced message points at `http://127.0.0.1:<port>`,
which resolves only on the machine that posted it. That is the local API
origin doing its job, not a bug to fix.

## Invariants

- **History is never announced.** The first pass writes
  `podrick_meta:announce_watermark` from the newest workout that already
  exists, announces nothing, and never moves it again. Only workouts strictly
  newer than the watermark are eligible. The watermark is a database row, so
  restarting cannot replay the archive.
  - This means the *first non-dry* run against a database is the one that
    matters. `--dry-run` deliberately does not seed it, so inspecting
    production is safe.
  - There is deliberately no backfill command. If one is ever wanted, it must
    be an explicit operation, not a flag on the normal path.
- **Nothing is announced twice.** `podrick_announcements` is keyed by workout
  id and claimed with `CREATE ONLY` *before* the Discord POST. Two workers, a
  redeploy mid-post, or a duplicated tick all converge on one message. The
  claim is taken conservatively: a lost race means "someone else has it," and
  a missed announcement is recoverable where a duplicate is not.
- **A crash between claiming and posting is retried.** A row with no
  `message_id` is an unconfirmed claim; the next pass re-posts it. `attempts`
  counts failures so a permanently poisoned message is visible in the data.
- **A deleted workout leaves its claim behind.** `just delete-lift` removes
  the `workouts` and `sets` rows and nothing in `podrick_*`, so correcting a
  lift (delete, then repaste) does not post a second message: the workout id
  is derived from the start timestamp, so the repaste reuses the id the claim
  already holds. The original message stays in the channel with whatever it
  said and a link that now 404s — fix or delete it in Discord by hand. If a
  claim was never posted, `tick` finds the workout gone and skips it rather
  than announcing a lift that no longer exists.
- **Only `source = 'manual'` workouts are announced.** CSV history does not
  join the homepage timeline or `/feed.xml` either, and a resync would
  otherwise replay years of workouts into the channel.
- **Podrick never writes a fitness table.** It reads `workouts` to decide what
  is eligible and owns the `podrick_*` tables; everything a message *says*
  comes from `GET /api/fitness/workouts/by-path/{path}`, so the Eastern
  projection, permanent paths, and derived records keep their single
  implementation in `lifting/archive`.
- **The site never writes a `podrick_*` table.** `/podrick` reads them through
  `status.rs` and renders fine when the database is unreachable or empty.
- **Message text escapes markdown.** Workout titles are user input as far as
  the channel is concerned, and `allowed_mentions` is empty so a title can
  never ping a server that isn't mine.
- **The message is a plain set list, never a highlight reel.** `announce.rs`
  deliberately mirrors `lifting/share.rs::share_text` line for line — same meta
  line, same `1. 135 lbs × 6 @ RPE 8` grammar, same blank-line grouping — with
  two deliberate divergences: personal records are omitted entirely (the
  permalink shows them properly, and four record categories per exercise is
  noise in a channel), and Discord needs a bold title, escaped markdown, and a
  2000-character cap. It is a separate implementation only because `share_text`
  reaches `WorkoutCard` → `filters` → the snapshot engine, none of which
  belongs in a bot. Change one format and consider the other.
- **Oversized workouts drop whole exercise blocks, never half of one**, and
  always keep the title, the facts line, and the permalink.

## Layout

```text
src/app/interests/podrick/
  mod.rs        the /podrick page (site binary)
  status.rs     the page's reads (site binary)
  db.rs         the worker's reads and writes (podrick binary)
  discord.rs    Discord REST client (podrick binary)
  announce.rs   job 1 (podrick binary)
  podrick.rs    [[bin]] podrick — CLI and poll loop
  models.rs     row structs, re-exported through src/data.rs
```

`db.rs`/`status.rs` are split so neither binary compiles the other's queries —
`just check` runs `clippy -D warnings`, and dead code fails it.

`podrick.rs` `#[path]`-includes `lifting/archive/eastern.rs` rather than
restating the permanent-path format, which is a public URL contract.

## Deployment

Podrick is a fourth Railway service built from the *same* `deploy/Dockerfile`
as the web service — `cargo build --release` produces every bin, and the image
copies `podrick` next to `benjisponge`. Set the service's Railway config path
to `/deploy/podrick.railway.toml`, which overrides the start command.

Set on the podrick service:

```text
SURREALDB_ENDPOINT=ws://surrealdb.railway.internal:8000
SURREALDB_NAMESPACE=benjisponge
SURREALDB_DATABASE=benjisponge
SURREALDB_USERNAME=${{surrealdb.SURREAL_USER}}
SURREALDB_PASSWORD=${{surrealdb.SURREAL_PASS}}
DISCORD_BOT_TOKEN=<from the developer portal>
PODRICK_LIFT_CHANNEL_ID=<channel id>
```

No `PORT`, no public domain, no Tunnel ingress. The service makes outbound
connections only.

Rollout order matters once: deploy the web service first so `src/schema.surql`
installs the `podrick_*` tables, then start podrick. Starting podrick against a
database that has never seen a workout would seed the watermark at the epoch
floor and announce the entire archive on the first real pass.

## Validation

```sh
just check
bash -n scripts/podrick-local.sh
bash -n scripts/dev.sh
just podrick-local once --dry-run   # local database, local API, no writes
```

For changes to the `just dev` supervision, also check that `--no-podrick`
starts no bot, that an unreachable port leaves the poller waiting instead of
failing, and that no `podrick` process survives the dev server — an orphan
here keeps posting to Discord.
