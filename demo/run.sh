#!/usr/bin/env bash
# Run a Hub in Docker, sign entries into a topic with the Producer, fetch the
# file and a heartbeat, and verify them with two independent verifiers. Needs
# only Docker and curl. Outputs land in demo/out/: demo.aof.jsonl,
# heartbeat.json, hub-key.txt.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"
OUT=$ROOT/demo/out
BASE=http://127.0.0.1:3717
TOPIC=demo
URL=$BASE/topics/$TOPIC
WRITE_TOKEN=demo-write-token
READ_TOKEN=demo-read-token

pass=0; fail=0
check() { # NAME GOT WANT
  if [ "$2" = "$3" ]; then echo "PASS $1"; pass=$((pass+1)); else echo "FAIL $1: got [$2] want [$3]"; fail=$((fail+1)); fi
}
die() { echo "ERROR: $*" >&2; exit 1; }
randhex() { head -c "$1" /dev/urandom | od -An -tx1 | tr -d ' \n'; }
# Run a command in the tools container, with demo/out mounted at /work.
in_tools() { docker compose run --rm --no-deps -T --user "$(id -u):$(id -g)" -v "$OUT:/work" tools "$@"; }
# Run the TypeScript verifier, with demo/out mounted read-only at /work.
in_ts() { docker compose run --rm --no-deps -T -v "$OUT:/work:ro" verify-ts "$@"; }
# A member's value from a flat JSON object on stdin.
member() { sed -n "s/.*\"$1\":\"\{0,1\}\([^\",}]*\).*/\1/p"; }

echo "== setup"
if [ ! -f .env ]; then
  printf 'HUB_KEY=%s\nHUB_ADMIN_TOKEN=%s\n' "$(randhex 32)" "$(randhex 16)" > .env
  echo "created .env with a random HUB_KEY and HUB_ADMIN_TOKEN"
fi
set -a; . ./.env; set +a
[ -n "${HUB_ADMIN_TOKEN:-}" ] || die ".env has no HUB_ADMIN_TOKEN"
mkdir -p "$OUT"

# Every image is built here, before any output is captured. `docker compose
# run` would otherwise build a missing image on first use and print the build
# log to stdout, into the file the next check reads.
echo "== build the images"
docker compose --profile tools build

echo "== start the Hub"
docker compose up -d --wait hub
HUB_KEY_PUB=$(docker compose logs --no-log-prefix hub | sed -n 's/^hub key \([A-Za-z0-9_-]*\)$/\1/p' | tail -n 1)
[ -n "$HUB_KEY_PUB" ] || die "the Hub's log has no 'hub key' line"
printf '%s\n' "$HUB_KEY_PUB" > "$OUT/hub-key.txt"
echo "Hub public key $HUB_KEY_PUB"

echo "== create the topic"
TOPIC_JSON="{\"topic_id\":\"$TOPIC\",\"heartbeat_interval_seconds\":5,\"heartbeat_hosts\":[],\"access\":[{\"token\":\"$WRITE_TOKEN\",\"role\":\"write\"},{\"token\":\"$READ_TOKEN\",\"role\":\"read\"}]}"
status=$(curl -s -o "$OUT/create.response" -w '%{http_code}' -H "Authorization: Bearer $HUB_ADMIN_TOKEN" --data-binary "$TOPIC_JSON" "$BASE/topics")
case "$status" in
  201) echo "created topic $TOPIC" ;;
  409) die "topic '$TOPIC' already exists in this Hub's data volume. Reset with: docker compose down -v && rm -rf demo/out" ;;
  *) die "creating the topic returned $status: $(cat "$OUT/create.response")" ;;
esac

echo "== sign and submit entries"
PRODUCER_KEY=$(randhex 32)
rm -rf "$OUT/rounds"; mkdir -p "$OUT/rounds"
next=1
for count in 1 3 5; do
  round=$OUT/rounds/from-$next
  mkdir "$round"
  prev=$(curl -sf "$URL/head" | member head_hash)
  in_tools producer mint "$PRODUCER_KEY" "$prev" "$next" "$count" "/work/rounds/from-$next"
  for p in "$round"/p-*; do
    code=$(curl -s -o /dev/null -w '%{http_code}' -X PUT -H "Authorization: Bearer $WRITE_TOKEN" --data-binary "@$p" "$URL/payloads/$(basename "$p")")
    [ "$code" = 201 ] || die "putting $(basename "$p") returned $code"
  done
  # A submission is its lines joined by 0x0a with no terminator.
  printf '%s' "$(cat "$round/lines.jsonl")" > "$round/body"
  code=$(curl -s -o "$round/receipt.json" -w '%{http_code}' --data-binary "@$round/body" "$URL/entries")
  [ "$code" = 201 ] || die "submitting $count entries returned $code: $(cat "$round/receipt.json")"
  echo "submitted $count entries: $(cat "$round/receipt.json")"
  next=$((next+count))
done
TOTAL=$((next-1))

echo "== fetch the file and the heartbeat"
curl -sf "$URL/aof" -o "$OUT/demo.aof.jsonl"
echo "waiting for a heartbeat at position $TOTAL"
got=
for _ in $(seq 1 60); do
  if curl -sf "$URL/heartbeat/latest" -o "$OUT/heartbeat.json" 2>/dev/null; then
    got=$(member position < "$OUT/heartbeat.json")
    [ "$got" = "$TOTAL" ] && break
  fi
  sleep 1
done
[ "$got" = "$TOTAL" ] || die "no heartbeat at position $TOTAL within 60 s (latest: [$got])"
echo "files in demo/out: demo.aof.jsonl heartbeat.json hub-key.txt"

# The verification section: every step reads only the three files in demo/out.
# Append further verifiers here, each as one or more check calls.
echo "== verify"
verdict=0
in_tools producer verify /work/demo.aof.jsonl > "$OUT/verdicts.txt" || verdict=$?
check "Haskell verifier: every line of demo.aof.jsonl is Sound" "$verdict" "0"
check "Haskell verifier: $TOTAL lines" "$(wc -l < "$OUT/verdicts.txt" | tr -d ' ')" "$TOTAL"
sig=$(in_tools python3 -I /opt/hub/bin/verify-signature.py "$(cat "$OUT/hub-key.txt")" /work/heartbeat.json)
check "heartbeat signature verifies under hub-key.txt (openssl)" "$sig" "Verified OK"
last=$(tail -n 1 "$OUT/verdicts.txt")
check "heartbeat head_hash equals the file's last line hash" "$(member head_hash < "$OUT/heartbeat.json")" "${last##* }"
check "heartbeat position equals the file's line count" "$(member position < "$OUT/heartbeat.json")" "$(wc -l < "$OUT/demo.aof.jsonl" | tr -d ' ')"

# The second verifier: TypeScript, written from the specification alone and
# sharing no code with the Haskell one. Agreement between the two is the point.
ts=0
in_ts aof /work/demo.aof.jsonl > "$OUT/verdicts-ts.jsonl" || ts=$?
check "TypeScript verifier: every line of demo.aof.jsonl is valid" "$ts" "0"
check "TypeScript verifier: $TOTAL lines" "$(wc -l < "$OUT/verdicts-ts.jsonl" | tr -d ' ')" "$TOTAL"
check "both verifiers give every line the same Event ID" \
  "$(sed -n 's/.*"event_id":"\([^"]*\)".*/\1/p' "$OUT/verdicts-ts.jsonl" | cksum)" "$(awk '{print $3}' "$OUT/verdicts.txt" | cksum)"
hb=0
in_ts heartbeat /work/demo.aof.jsonl /work/heartbeat.json --hub-key "$(cat "$OUT/hub-key.txt")" --topic "$TOPIC" > "$OUT/heartbeat-ts.json" || hb=$?
check "TypeScript verifier: the file matches the Hub's heartbeat" "$(member outcome < "$OUT/heartbeat-ts.json")" "match"

echo "== result: $pass passed, $fail failed"
[ "$fail" = 0 ]
