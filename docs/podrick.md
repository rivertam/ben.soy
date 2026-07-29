# Podrick

A Discord bot for a server I'm in, deployed from this repo. Read this before
changing anything under `src/app/interests/podrick/`.

- **Job 1:** announce a lift in a channel when one is published here.
- **Job 2:** sync Pants Off posts from `#no-pants-talk`, classify and display
  them as three heatmaps, report infarctions, and worm completed kwerms. Its
  first run seeds the database from the channel's complete history without
  replying to any historical post.

The bot is the `podrick` binary. `/podrick` is a login-gated hidden page:
grant and revoke it per email at `/admin/permissions` (`docs/auth.md`).

## Why there is no gateway and no bot framework

Both jobs are REST shaped. Announcing and infarction notices are
`POST /channels/{id}/messages`; worming is an idempotent
`PUT /channels/{id}/messages/{id}/reactions/{emoji}/@me`. Reading a channel —
live and while seeding history — is the same `GET /channels/{id}/messages`
walker, paged backwards by snowflake. A gateway connection would buy
sub-second latency in exchange for a heartbeat/resume lifecycle and a second
ingestion path that could drift from the backfill.

Discord can hide message *content* from REST responses when Message Content is
not enabled, but Pants Off does not read content: its whole source fact is
message id, author id, and creation timestamp. Leave every Privileged Gateway
Intent off. The bot role needs View Channel and Read Message History in the
Pants and infarctions channels, Add Reactions for the first worm, and Send
Messages in both output channels. Reading the infarctions channel is how an
uncertain POST is reconciled without duplicating it after a long outage.

## Setting up the Discord side

1. https://discord.com/developers/applications → **New Application**, name it
   Podrick.
2. **Bot** → **Reset Token** → copy it. This is `DISCORD_BOT_TOKEN` and is
   shown once. Leave every Privileged Gateway Intent off.
3. **OAuth2 → URL Generator** → scope `bot`, permissions **View Channel**,
   **Send Messages**, **Read Message History**, and **Add Reactions**. Open the
   generated URL and install it to the server.
   - Installing requires **Manage Server** on that guild. If you do not have
     it, the owner must open the URL; there is no way around this.
4. In Discord, **Settings → Advanced → Developer Mode** on, then copy the
   three channel ids:
   - lift announcements → `PODRICK_LIFT_CHANNEL_ID`
   - `#no-pants-talk` → `PODRICK_PANTS_CHANNEL_ID`
   - `#discord-infarctions` → `PODRICK_INFARCTIONS_CHANNEL_ID`
5. Confirm the bot can actually see all configured channels. A private
   channel needs the bot's role added to it explicitly — installing to the
   guild is not enough, and this is the usual cause of a 403.

## Running it

```sh
just podrick once --dry-run          # render to stdout, post nothing
just podrick once                    # one pass
just podrick run --interval 30       # poll forever
just podrick --help
```

`--dry-run` is read-only: it writes neither database nor Discord — no
watermarks, cursors, claims, reactions, posts, or attempt counters. The lift
job still needs no token, so its copy can be developed before the Discord
application exists. When the Pants channel is configured a token is required
even for dry-run because reading history is an authenticated Discord call.
That read-only guarantee is deliberate: both seed boundaries are once-ever
decisions and a "just looking" run against production must not make them.

Each job is enabled by its channel variables; Pants Off requires its source
and infarctions variables as a pair. At least one job must be configured. The
token comes from `--token`, else `$DISCORD_BOT_TOKEN`, else
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
just dev --podrick-reset             # clear all local Podrick state, then start
just podrick-local once --dry-run    # one read-only pass, beside --no-podrick
```

It starts only when `.env.dev` names a Podrick channel *and* a token resolves.
With no channel configured `just dev` behaves exactly as it always has and
says nothing about Podrick; with a channel but no token it says so once and
starts the site anyway. Discord is not local: use test channels in `.env.dev`.
In particular, configuring the production Pants source locally would read it
and could produce live reactions/notices backed by the local outbox.

`scripts/podrick-local.sh` sources `.env.dev` for the channel ids (and
`DISCORD_BOT_TOKEN`, if you keep it there), then pins the database and `--api`
to the local stack *after* sourcing — the same ordering, for the same reason,
as `scripts/dev.sh`: a stray production endpoint in `.env.dev` must not be
able to point the bot at the real database. `dev.sh` waits for the site to
answer before starting it, and a configured lift job refuses to start by hand
when no site answers, because otherwise a cold build looks like a run of
announce failures.

**The watermark is why a lift you already have does not announce.** The first
run seeds it at the newest workout that already exists, so anything pasted
after Podrick starts is announced and anything already there never is —
including, exactly, the workout it seeded from, since eligibility is *strictly*
newer. The seeding log reports UTC and `/lifting` shows Eastern, so the lift
sitting on the boundary does not look like the watermark unless you convert. To
re-announce a lift you already have locally: delete it (the delete control on
`/lifting`, or `just delete-lift`), then `just dev --podrick-reset`, then
paste it again. `--podrick-reset` clears every local `podrick_*` row, including
the local Pants history and action outbox; it exists only on the dev scripts
and deliberately not on the binary, because production must not grow a reset
or backfill switch (see Invariants). When `.env.dev` sets `PODRICK_SYNC_TOKEN`
(and either omits Pants or points it at the production channel), the next
empty-state pass pulls `GET /api/podrick/seed` — every production
`podrick_announcements`, `podrick_pants_messages`, `podrick_pants_actions`, and
`podrick_meta` row — so local mirrors production without re-walking Discord or
re-seeding the lift watermark from the local archive. Test Pants channels skip
that path and rebuild as before.

Run `just podrick-local` by hand only alongside `just dev --no-podrick`, or
two bots poll the same database. Multiple pollers are unsupported: claims
dedupe first ownership, but both workers can retry the same unfinished lift
claim. A `--reset` also mutates their shared local database underneath the
running process.

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
- **Each lift is claimed once.** `podrick_announcements` is keyed by workout
  id and claimed with `CREATE ONLY` *before* the Discord POST. That prevents
  two workers from first owning the same lift. Keep one worker replica:
  concurrent retries of an already-unconfirmed lift claim are not coordinated.
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
- **Pants history is source facts, not mutable claim totals.**
  `podrick_pants_messages` stores one recognized participant message per
  Discord snowflake: channel, author, and creation second. Classification and
  daily aggregation are shared Rust code used by both worker and page, so the
  database never grows a stale "claims" row. Edits do not matter because
  content is unused; a later Discord deletion deliberately does not rewrite
  the historical event.
  - The participant allowlist is deliberately code, not display-name matching:
    Zack `284908269649002506`, Dr. Angor `224178544379428864`, and Captain
    Beyond Beefheart `129076065074151424`. A nickname change cannot change
    ownership of history.
  - The first non-dry run binds `PODRICK_PANTS_CHANNEL_ID` into
    `podrick_meta:pants_source_channel`. Treat that channel as immutable:
    changing it without migrating the stored facts and cursors would combine
    two histories. `just dev --podrick-reset` is the local way to switch test
    channels; a production move requires an explicit migration with Podrick
    stopped.
- **The canonical Pants clock is `America/New_York`.** Any second within the
  6:07 AM or 6:07 PM minute claims that slot. A participant's repeated posts
  in one slot remain source rows but make one claim; both distinct slots make
  two claims. Another hour at minute 07 is out of town and makes no claim; any
  other minute is an infarction. Jiff's bundled IANA data owns EST/EDT
  transitions.
- **The heatmaps and leaderboards are calendar-year projections.** `/podrick`
  is the current Eastern year and `?year=YYYY` selects an earlier tracked
  year. Each grid runs from January 1 through December 31, padded only to
  complete Sunday-to-Saturday weeks; padding, pre-tracking dates, and future
  dates never enter totals. The three boards independently rank claims,
  doubles, and consecutive dates with at least one claim. Streaks reset on
  January 1, and missing, out-of-town-only, or infarction-only dates break
  them. Kwerms and asynkwerms are crew totals rather than individual ranks.
- **Kwerms are set intersections, not message counts.** All three sharing AM
  makes an AM kwerm and all three sharing PM makes a PM kwerm, so three doubles
  can make two kwerms. If everyone has at least one slot but the three-way
  intersection is empty, the day is an asynkwerm. An asynkwerm is not acted on
  until 6:08 PM Eastern, when the final valid minute has closed.
- **The history seed is silent and resumable.** The first Pants pass captures
  the channel head, walks backwards in 100-message pages, and checkpoints its
  exclusive `before` snowflake. A restart or rate limit resumes the walk.
  Source rows are written, but no historical infarction post or worm reaction
  is claimed. Only after the oldest page does `pants_cursor` establish live
  mode; messages posted during the walk are then picked up beyond the original
  head.
  - Locally, an empty `podrick_*` database may instead install production's
    full snapshot from `GET /api/podrick/seed` when `PODRICK_SEED_URL` and
    `PODRICK_SYNC_TOKEN` are set (see Running it → Locally). That path writes
    announcements, Pants facts, actions, and meta, then continues normally;
    it is not a production reset or backfill switch.
- **A live cursor advances only after facts and action claims are durable.**
  Live polling also walks newest-to-oldest until it crosses the numeric
  snowflake cursor, so a burst over 100 messages cannot create a gap. Unknown
  authors still advance the cursor but never enter a Pants table.
- **Pants side effects are an outbox.** Deterministic
  `podrick_pants_actions` keys claim one infarction notice per source message
  and one worm reaction per qualifying participant message. Reactions are
  idempotent Discord PUTs; a 404 means the source message was deleted, so that
  reaction is terminally skipped instead of poisoning the queue. Infarction
  posts use a stable Discord nonce with `enforce_nonce`, and every retry first
  scans output history back to the action's claim time for that nonce because
  Discord's own uniqueness window lasts only a few minutes. Incomplete rows
  retry on later passes, failures increment `attempts`, and a rate limit delays
  the next pass for Discord's full `Retry-After`.
- **`/podrick` is hidden, not merely unlinked.** It stays out of `INTERESTS`
  and `site_routes()`, repeats the database grant check on every request,
  disables public analytics, and emits `no-store` before every rendered shell.
  Its only listing and grant form come from `access::HIDDEN_PAGES`.

## Layout

```text
src/app/interests/podrick/
  mod.rs        the /podrick page (site binary)
  heatmap.rs    annual Pants Off heatmaps and leaderboards (site binary)
  status.rs     the page's reads (site binary)
  seed.rs       GET /api/podrick/seed (site binary)
  db.rs         the worker's reads and writes (podrick binary)
  discord.rs    Discord REST client (podrick binary)
  announce.rs   job 1 (podrick binary)
  pants.rs      job 2 history/live ingestion and actions (podrick binary)
  podrick.rs    [[bin]] podrick — CLI and poll loop
  models.rs     row structs and shared Pants rules, re-exported through data.rs
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
PODRICK_PANTS_CHANNEL_ID=883473115085164544
PODRICK_INFARCTIONS_CHANNEL_ID=1049738190107451433
```

No `PORT`, no public domain, no Tunnel ingress. The service makes outbound
connections only.

Before Podrick's first non-dry run, make sure the existing workout archive is
already imported. An empty database seeds the lift watermark at the epoch
floor, so importing manual lifts afterward would announce the import. Both the
web and Podrick binaries reconcile `src/schema.surql` when they first connect;
deploying the web service first is still the clearest rollout because it makes
the hidden page available before the history seed starts. Because `/podrick`
used to be public and cacheable, ship the web change with `just deploy` (or
otherwise purge the Cloudflare zone) so an old public response cannot remain at
the edge. The first Pants run can take several ticks because it intentionally
caps history work at five pages per pass. Wait for either `pants-history` with
`complete: true` or `pants-history-complete` before expecting live infarctions
or worms.

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
