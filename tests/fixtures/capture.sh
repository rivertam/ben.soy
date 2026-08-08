#!/usr/bin/env bash
# Golden-fixture capture for the Rust data-layer contract.
#
# Captures the live TypeScript-Worker API responses (bodies byte-exact,
# selected header dumps, HTTP statuses) plus remote D1 dumps. These fixtures
# ARE the contract for the Rust port: there is no Worker test suite.
#
# Usage: bash tests/fixtures/capture.sh [origin]
#   origin defaults to https://benjisponge.com
#
# Historical only: it overwrites the fixtures and depends on the retired
# Worker/D1 data path. Do not run it against the current production service.

set -euo pipefail

origin="${1:-https://benjisponge.com}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
api_dir="$here/api"
d1_dir="$here/d1"
manifest="$api_dir/manifest.tsv"

mkdir -p "$api_dir" "$d1_dir"
: >"$manifest"

# capture <name> <method> <path-and-query> [--headers]
# Records: body -> api/<name>.json, status -> manifest, optional headers.
capture() {
  local name="$1" method="$2" path="$3" want_headers="${4:-}"
  local url="$origin$path"
  local body="$api_dir/$name.json"
  local status
  local -a args=(-sS -o "$body" -w '%{http_code}' -X "$method" --max-time 30)
  if [[ "$want_headers" == "--headers" ]]; then
    args+=(-D "$api_dir/$name.headers")
  fi
  status=$(curl "${args[@]}" "$url")
  printf '%s\t%s\t%s\t%s\n' "$name" "$status" "$method" "$path" >>"$manifest"
  echo "  $status $name"
}

echo "== fitness reads =="
capture facets GET "/api/fitness/facets"
capture calendar GET "/api/fitness/calendar"
capture latest GET "/api/fitness/workouts/latest" --headers
capture ids GET "/api/fitness/ids"
capture sets_default GET "/api/fitness/sets" --headers
capture sets_page2 GET "/api/fitness/sets?page=2"
capture sets_perpage40 GET "/api/fitness/sets?per_page=40"
capture sets_perpage10 GET "/api/fitness/sets?per_page=10"
capture sets_movement GET "/api/fitness/sets?movement=squat-type"
capture sets_movement_multi GET "/api/fitness/sets?movement=squat-type&movement=hinge-type"
capture sets_movement_muscle GET "/api/fitness/sets?movement=squat-type&muscle=quads"
capture sets_q GET "/api/fitness/sets?q=squat"
capture sets_q_case GET "/api/fitness/sets?q=SQUAT"
capture sets_exercise GET "/api/fitness/sets?exercise=Squat%20(Barbell)"
capture sets_exercise_case GET "/api/fitness/sets?exercise=squat%20(barbell)"
capture sets_dates GET "/api/fitness/sets?from=2024-01-01&to=2024-12-31"
capture sets_weekday GET "/api/fitness/sets?weekday=mon"
capture sets_timeofday GET "/api/fitness/sets?time_of_day=morning"
capture sets_timeofday_night GET "/api/fitness/sets?time_of_day=night"
capture sets_load GET "/api/fitness/sets?min_load=225&max_load=315"
capture sets_reps GET "/api/fitness/sets?min_reps=5&max_reps=8"
capture sets_effort GET "/api/fitness/sets?max_effort=8.5"
capture sets_has_record GET "/api/fitness/sets?has_record=true"
capture sets_has_record_f GET "/api/fitness/sets?has_record=false"
capture sets_has_superset GET "/api/fitness/sets?has_superset=true"
capture sets_has_notes GET "/api/fitness/sets?has_notes=true"
capture sets_incomplete GET "/api/fitness/sets?incomplete=true"
capture sets_duration_susp GET "/api/fitness/sets?duration=suspicious"
capture sets_settype GET "/api/fitness/sets?set_type=FAILURE_SET"
capture sets_settype_multi GET "/api/fitness/sets?set_type=WARMUP_SET&set_type=DROP_SET"
capture sets_combo1 GET "/api/fitness/sets?q=press&movement=press-type&from=2025-01-01&min_reps=3"
capture sets_combo2 GET "/api/fitness/sets?muscle=quads&equipment=barbell&per_page=40&page=2"
capture sets_empty GET "/api/fitness/sets?from=2019-01-01&to=2019-12-31"

echo "== fitness by-path =="
real_path=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["workout"]["path"])' "$api_dir/latest.json")
capture by_path_real GET "/api/fitness/workouts/by-path/$real_path"
capture by_path_missing GET "/api/fitness/workouts/by-path/2020-01-01T00-00-00-05-00" --headers
capture by_path_malformed GET "/api/fitness/workouts/by-path/not-a-path"
capture by_path_bad_offset GET "/api/fitness/workouts/by-path/2024-06-01T10-00-00-07-00"

echo "== fitness errors =="
capture err_unknown GET "/api/fitness/sets?bogus=1" --headers
capture err_page0 GET "/api/fitness/sets?page=0"
capture err_page_dup GET "/api/fitness/sets?page=1&page=2"
capture err_perpage15 GET "/api/fitness/sets?per_page=15"
capture err_perpage50 GET "/api/fitness/sets?per_page=50"
capture err_from_feb30 GET "/api/fitness/sets?from=2026-02-30"
capture err_from_gt_to GET "/api/fitness/sets?from=2025-01-01&to=2024-01-01"
capture err_weekday GET "/api/fitness/sets?weekday=monday"
capture err_timeofday GET "/api/fitness/sets?time_of_day=noon"
capture err_q_long GET "/api/fitness/sets?q=$(python3 -c 'print("a"*101)')"
capture err_q_50byte GET "/api/fitness/sets?q=$(python3 -c 'import urllib.parse; print(urllib.parse.quote("\U0001F4AA"*13))')"
capture err_movement_nine GET "/api/fitness/sets?movement=a&movement=b&movement=c&movement=d&movement=e&movement=f&movement=g&movement=h&movement=i"
capture err_movement_bad GET "/api/fitness/sets?movement=SQUAT"
capture err_movement_dup GET "/api/fitness/sets?movement=squat-type&movement=squat-type"
capture err_exercise_dup GET "/api/fitness/sets?exercise=a&exercise=b"
capture err_settype_bad GET "/api/fitness/sets?set_type=working"
capture err_minload_text GET "/api/fitness/sets?min_load=abc"
capture err_minload_gt_max GET "/api/fitness/sets?min_load=300&max_load=200"
capture err_effort_places GET "/api/fitness/sets?max_effort=8.555"
capture err_facets_qs GET "/api/fitness/facets?x=1"
capture err_calendar_qs GET "/api/fitness/calendar?x=1"
capture err_latest_qs GET "/api/fitness/workouts/latest?x=1"
capture err_ids_qs GET "/api/fitness/ids?x=1"
capture import_unauth POST "/api/fitness/import" --headers
capture fitness_404 GET "/api/fitness/nope"

echo "== spire =="
capture spire_runs GET "/api/spire/runs" --headers
capture spire_ids GET "/api/spire/ids"
capture spire_404 GET "/api/spire/nope"
capture spire_post_unauth POST "/api/spire/runs" --headers

echo "== D1 dumps (remote, read-only) =="
d1() {
  local name="$1" sql="$2"
  (cd "$here/../../deploy" && npx wrangler d1 execute benjisponge-spire \
    --remote --json --command "$sql") >"$d1_dir/$name.json"
  echo "  d1 $name"
}
# workout_triples is the Eastern-projection corpus, consumed by
# crates/diary-core's eastern tests — captured straight into that crate.
diary_d1_dir="$here/../../crates/diary-core/tests/fixtures/d1"
mkdir -p "$diary_d1_dir"
(cd "$here/../../deploy" && npx wrangler d1 execute benjisponge-spire \
  --remote --json --command "SELECT started_at_utc, started_at_local, eastern_offset_minutes, duration_seconds, id FROM workouts ORDER BY id") >"$diary_d1_dir/workout_triples.json"
echo "  d1 workout_triples (-> crates/diary-core)"
d1 spire_rows "SELECT id, date, start_time, character, win, abandoned, ascension, acts, floors, killed_by, kill_kind, run_time, seed, game_mode, build_id, added_at FROM spire_runs ORDER BY id"
d1 meta_versions "SELECT 'fitness' AS src, v FROM fitness_meta WHERE k='version' UNION ALL SELECT 'spire', v FROM spire_meta WHERE k='version'"
d1 counts "SELECT (SELECT COUNT(*) FROM workouts) AS workouts, (SELECT COUNT(*) FROM sets) AS sets, (SELECT COUNT(*) FROM exercises) AS exercises, (SELECT COUNT(*) FROM exercise_tags) AS tags, (SELECT COUNT(*) FROM set_records) AS set_records, (SELECT COUNT(*) FROM spire_runs) AS spire_runs"

echo "done: $(wc -l <"$manifest") API captures + 4 D1 dumps"
