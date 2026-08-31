# Health Connect step sync

Daily steps flow through Android rather than Garmin's partner-only Health API:

```text
Garmin watch -> Garmin Connect -> Health Connect -> Health.md
             -> POST /api/fitness/steps -> daily_steps -> /fitness
```

Garmin Connect's Health Connect integration is one-way and writes steps after
a successful device sync. Health.md reads Health Connect's daily aggregate, so
Health Connect's source-priority rules resolve overlapping phone/watch records
before the site sees one number. Relevant upstream documentation:

- [Garmin: sharing Garmin Connect data with Health Connect](https://support.garmin.com/en-US/?faq=JToBEy0jfe6pIygark2Ui5)
- [Android: data sources and priority](https://support.google.com/android/answer/12990553?hl=en)
- [Health.md Android app](https://play.google.com/store/apps/details?id=com.healthmd.android)
- [Health.md API endpoint contract](https://github.com/CodyBontecou/health-md-android/blob/main/docs/api-endpoint-export.md)

## One-time site setup

Generate a dedicated secret. Do not reuse `FITNESS_SYNC_TOKEN`: the steps
token is stored on the phone and can only reach this bounded upsert.

```sh
openssl rand -hex 32
```

In Railway, open the `ben.soy` web service, add the generated value as
`STEPS_SYNC_TOKEN`, and deploy the commit containing the endpoint. Save the
value in a password manager long enough to enter it into Health.md. The local
`just dev` stack uses `local-development` automatically.

The first `GET /api/fitness/steps` after deploy connects to SurrealDB and
reconciles the `daily_steps` schema. A clean response is expected before the
phone has uploaded anything:

```sh
curl --fail-with-body https://ben.soy/api/fitness/steps
# {"days":[]}
```

## Garmin and Health Connect

Garmin's native Health Connect sharing currently requires Android 14 or newer.

1. Update Garmin Connect, sync the watch successfully, then open **Garmin
   Connect -> More -> Settings -> Connected Apps -> Health Connect**. Enable
   sharing and allow **Steps**. Garmin does not read back from Health Connect.
2. Open Health Connect. On Android 14+, search phone Settings for **Health
   Connect** (the usual path is **Security & privacy -> Privacy controls ->
   Health Connect**).
3. Open **Manage data -> Data sources and priority -> Activity/Steps**. Make
   Garmin Connect the top step source. Remove the phone or another watch from
   the step total if it represents the same walking; otherwise overlapping
   sources can produce a surprising aggregate.
4. Check that yesterday has a plausible step total in Health Connect. If it
   does not, open Garmin Connect and force one more watch sync before debugging
   the exporter.

## Health.md

Install Health.md from Google Play. Manual exports can test the complete path;
scheduled automation is a one-time in-app purchase, not a subscription.

1. Select **Health Connect** as the provider and grant read access to **Steps**.
   Grant historical access for the initial backfill and background access when
   the Schedule screen asks for it.
2. In metric selection, keep only **Steps**. Keep the date format at the default
   ISO `YYYY-MM-DD` and turn **Detailed Time-Series** off. The site wants one
   aggregate integer per calendar day and discards every other health metric.
3. Under **Export**, select **Compatibility Export**, then **Export Target ->
   API endpoint**.
4. Set the exact URL to `https://ben.soy/api/fitness/steps`. Use the canonical
   HTTPS host with no trailing redirect: compatibility redirects can change a
   POST into a GET.
5. Paste the generated secret into **Bearer Token or Basic Credential**.
   Paste only the token; Health.md adds `Bearer ` automatically and stores the
   credential with Android Keystore-backed encrypted preferences.
6. Manually export **Yesterday**. Then open the public read endpoint and the
   fitness page:

   ```sh
   curl --fail-with-body https://ben.soy/api/fitness/steps
   ```

   A successful first import returns a receipt in Health.md with one `added`
   row. The GET returns newest-first `{date,steps}` rows, and `/fitness#steps`
   renders the recent chart.
7. Once that works, manually export the last 30 or 90 days for the initial
   backfill. Health Connect can only return history it actually has and that
   Health.md is allowed to read, so enabling Garmin sharing today may not make
   older Garmin days appear.
8. In **Schedule**, select **API endpoint**, **Past complete days**, daily at a
   convenient morning time, and a seven-day lookback. Enable the schedule and
   allow Alarms & reminders/background access if Android prompts. Re-exporting
   seven days is cheap and picks up a Garmin sync that arrived late.

Health.md treats any `2xx` as delivered. It retries network errors, 408/429,
and server `5xx` responses; ordinary `4xx` responses require correcting the
configuration. Its History screen shows failed dates and supports explicit
retry.

## Endpoint contract

`POST /api/fitness/steps` requires:

- `Authorization: Bearer $STEPS_SYNC_TOKEN`
- `Content-Type: application/json`
- Health.md compatibility envelope `healthmd.api_export` v1 containing daily
  record schema `healthmd.health_data` v4
- at most 400 records/failed dates and 256 KiB total body size
- ISO calendar dates, nonnegative integer steps no greater than 1,000,000, and
  no duplicate day in one request

Unknown health fields are ignored, but the configured exporter should still
select Steps only so unrelated private data never leaves the phone. A record
without steps is reported as `omitted`; a failed date never deletes an existing
row. The response is:

```text
{received,accepted,added,updated,unchanged,stale,omitted,failed_dates}
```

One deterministic `daily_steps:{YYYY-MM-DD}` row owns each calendar date.
Later exports replace its total because Health Connect can incorporate late
watch data or a priority correction. The Health.md `exported_at` watermark is
stored in milliseconds; an older delayed request is accepted as `2xx` but
counted `stale` and cannot regress the row. Steps do not bump
`fitness_meta:version` and never enter lifting records, volume, filters, the
training heatmap, running activities, or Podrick.

`GET /api/fitness/steps` is public, `no-store`, CORS-readable, accepts no query
parameters, and returns at most 35 newest days as
`{"days":[{"date":"YYYY-MM-DD","steps":12345}]}`.

## Troubleshooting

- `401 unauthorized`: Railway and Health.md do not hold the same
  `STEPS_SYNC_TOKEN`, or the Authorization field was entered as a Basic value.
- `400`: use Compatibility Export, ISO dates, and the current Health.md daily
  v4 contract. Raw API Snapshot is intentionally unsupported.
- `413`: Detailed Time-Series or many unrelated metrics were probably enabled;
  select only Steps and turn detailed data off.
- `500`: the site could not reach or write SurrealDB. Health.md will retry a
  scheduled export; check the web-service logs before manually retrying.
- A successful receipt but no new day: inspect `omitted`, `failed_dates`, and
  `stale`, then verify the date in Health Connect itself.
- A wrong/doubled total: fix **Health Connect -> Manage data -> Data sources and
  priority**. The site intentionally trusts Health Connect's aggregate and
  does not attempt a second source-level deduplication.

To rotate the secret, replace `STEPS_SYNC_TOKEN` in Railway, deploy, then
replace Health.md's saved Authorization value. Scheduled failures remain in
Health.md history until the endpoint credentials are corrected and explicitly
retried.
