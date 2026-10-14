#!/bin/zsh
# A real Producer against the Hub: mint with the upstream eventchain Producer,
# put payloads, submit, distribute, verify the served file with the upstream
# Verifier, restart, repeat. Run from anywhere; needs the Hub and the producer
# built (README, "The end-to-end run"). Every check prints PASS or FAIL and
# the last line counts them. The Hub's data lives in a fresh temporary
# directory that is left behind for inspection.
set -u
ROOT=${0:A:h:h:h}
cd $ROOT
P=$(cd tools/producer && cabal list-bin producer)
VERIFY_SIG=$ROOT/tools/producer/verify-signature.py
WORK=$(mktemp -d /tmp/hub-producer.XXXXXX)
DATA=$WORK/data; OUT=$WORK/out; LOG=$WORK/hub.log
PORT=${HUB_E2E_PORT:-3750}; BASE=http://127.0.0.1:$PORT; URL=$BASE/topics/e2e
mkdir -p $DATA $OUT
ADMIN=admin-$(openssl rand -hex 8)
export HUB_KEY=$(openssl rand -hex 32) HUB_DATA=$DATA HUB_ADDR=127.0.0.1:$PORT
PRODUCER_KEY=$(openssl rand -hex 32)
echo "work directory $WORK"

pass=0; fail=0
check() { if [ "$2" = "$3" ]; then echo "PASS $1"; pass=$((pass+1)); else echo "FAIL $1: got [$2] want [$3]"; fail=$((fail+1)); fi; }
# Poll a condition every 0.1 s, up to 10 s.
waitfor() { for i in {1..100}; do eval "$1" && return 0; sleep 0.1; done; return 1; }
code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
start() { HUB_ADMIN_TOKEN=$ADMIN ./target/release/hub >> $LOG 2>&1 & PID=$!; waitfor "curl -s -o /dev/null $BASE/topics/x/head || ! kill -0 $PID 2>/dev/null"; }
stop() { kill $PID 2>/dev/null; wait $PID 2>/dev/null; }
field() { python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$1" "$2"; }
sha256_b64url() { python3 -I -c 'import sys,hashlib,base64; print(base64.urlsafe_b64encode(hashlib.sha256(open(sys.argv[1],"rb").read()).digest()).rstrip(b"=").decode())' "$1"; }
W="Authorization: Bearer w-secret"
head_hash() { curl -s $URL/head | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["head_hash"])'; }
# mint FIRST COUNT PREV DIR: payloads and lines into DIR
mint() { mkdir -p $4; $P mint $PRODUCER_KEY $3 $1 $2 $4; }
# put every payload of DIR; print the number of 201s
put_payloads() { local n=0; for f in $1/p-*; do [ "$(code -X PUT -H "$W" --data-binary @$f $URL/payloads/$(basename $f))" = 201 ] && n=$((n+1)); done; echo $n; }
# body DIR: the lines joined by 0x0a with no terminator, as a submission is
body() { python3 -I -c 'import sys; sys.stdout.buffer.write(open(sys.argv[1],"rb").read().rstrip(b"\n"))' $1/lines.jsonl > $1/body; echo $1/body; }
# submit DIR OUTFILE: print the status
submit() { curl -s -o $2 -w '%{http_code}' --data-binary @$(body $1) $URL/entries; }

start
HUBKEY=$(grep '^hub key' $LOG | awk '{print $3}')
echo "== topic"
check "create e2e" "$(code -X POST -H "Authorization: Bearer $ADMIN" --data-binary '{"topic_id":"e2e","heartbeat_interval_seconds":2,"heartbeat_hosts":[],"access":[{"token":"w-secret","role":"write"},{"token":"r-secret","role":"read"}]}' $BASE/topics)" "201"
GENESIS=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
check "head is genesis" "$(head_hash)" "$GENESIS"

echo "== round 1: one entry from genesis"
mint 1 1 $GENESIS $OUT/r1
check "payload put" "$(put_payloads $OUT/r1)" "1"
check "submit 201" "$(submit $OUT/r1 $OUT/r1.receipt)" "201"
H1=$(tail -1 $OUT/r1/hashes)
check "receipt head_hash is the Producer's line hash" "$(field $OUT/r1.receipt head_hash)" "$H1"
check "receipt position 1" "$(field $OUT/r1.receipt position)" "1"
check "receipt count 1" "$(field $OUT/r1.receipt count)" "1"
check "one-line submission_hash equals the Event ID" "$(field $OUT/r1.receipt submission_hash)" "$H1"
check "receipt signature verifies under the hub key" "$(python3 -I $VERIFY_SIG $HUBKEY $OUT/r1.receipt)" "Verified OK"
check "head moved" "$(head_hash)" "$H1"

echo "== round 2: five entries chained from the live head, one submission"
mint 2 5 "$(head_hash)" $OUT/r2
check "payloads put" "$(put_payloads $OUT/r2)" "5"
check "submit 201" "$(submit $OUT/r2 $OUT/r2.receipt)" "201"
H6=$(tail -1 $OUT/r2/hashes)
check "receipt head_hash" "$(field $OUT/r2.receipt head_hash)" "$H6"
check "receipt position 6" "$(field $OUT/r2.receipt position)" "6"
check "receipt count 5" "$(field $OUT/r2.receipt count)" "5"
check "submission_hash is sha256 of the body" "$(field $OUT/r2.receipt submission_hash)" "$(sha256_b64url $OUT/r2/body)"

echo "== round 3: a Producer with a stale head"
mint 7 1 $H1 $OUT/r3
check "submit 409" "$(submit $OUT/r3 $OUT/r3.refusal)" "409"
check "reason head_mismatch" "$(field $OUT/r3.refusal reason)" "head_mismatch"
check "refusal carries the current head" "$(field $OUT/r3.refusal head_hash)" "$H6"
check "refusal carries the current position" "$(field $OUT/r3.refusal position)" "6"
check "refusal signature verifies" "$(python3 -I $VERIFY_SIG $HUBKEY $OUT/r3.refusal)" "Verified OK"
check "nothing appended" "$(head_hash)" "$H6"

echo "== round 4: a batch whose second line was altered after signing"
mint 7 3 $H6 $OUT/r4
python3 -I -c '
import sys
p=sys.argv[1]; lines=open(p,"rb").read().split(b"\n")
l=bytearray(lines[1]); i=l.index(b"\"entry_id\":\"")+len(b"\"entry_id\":\""); l[i]=ord("X"); lines[1]=bytes(l)
open(p,"wb").write(b"\n".join(lines))' $OUT/r4/lines.jsonl
check "submit 409" "$(submit $OUT/r4 $OUT/r4.refusal)" "409"
check "reason signature_invalid" "$(field $OUT/r4.refusal reason)" "signature_invalid"
check "line 1" "$(field $OUT/r4.refusal line)" "1"
check "nothing appended" "$(head_hash)" "$H6"

echo "== round 5: ten more, one submission"
mint 7 10 $H6 $OUT/r5
check "payloads put" "$(put_payloads $OUT/r5)" "10"
check "submit 201" "$(submit $OUT/r5 $OUT/r5.receipt)" "201"
H16=$(tail -1 $OUT/r5/hashes)
check "position 16" "$(field $OUT/r5.receipt position)" "16"

echo "== distribute and verify with the upstream Verifier"
cat $OUT/r1/lines.jsonl $OUT/r2/lines.jsonl $OUT/r5/lines.jsonl > $OUT/expected.jsonl
cat $OUT/r1/hashes $OUT/r2/hashes $OUT/r5/hashes > $OUT/expected.hashes
curl -s $URL/aof > $OUT/aof.jsonl
check "the served AOF is byte-identical to what the Producer wrote" "$(cmp $OUT/aof.jsonl $OUT/expected.jsonl && echo same)" "same"
check "the file on disk is too" "$(cmp $DATA/e2e/aof.jsonl $OUT/expected.jsonl && echo same)" "same"
$P verify $OUT/aof.jsonl > $OUT/verdicts; check "upstream verify: every line Sound" "$?" "0"
check "16 verdicts" "$(wc -l < $OUT/verdicts | tr -d ' ')" "16"
check "verifier hashes equal Producer hashes" "$(awk '{print $3}' $OUT/verdicts | cmp - $OUT/expected.hashes && echo same)" "same"
curl -s "$URL/entries?after=$H1" > $OUT/ext.jsonl
check "extension after line 1 is lines 2..16" "$(tail -n +2 $OUT/expected.jsonl | cmp - $OUT/ext.jsonl && echo same)" "same"
check "extension after the head is empty" "$(curl -s "$URL/entries?after=$H16" | wc -c | tr -d ' ')" "0"
H7=$(sed -n 1p $OUT/r5/hashes)
curl -s $URL/lookup/$H7 > $OUT/lookup.json
check "lookup position 7" "$(field $OUT/lookup.json position)" "7"
check "lookup line is line 7" "$(python3 -I -c 'import json,sys; sys.stdout.buffer.write(json.load(open(sys.argv[1]))["line"].encode())' $OUT/lookup.json | cmp - <(sed -n 7p $OUT/expected.jsonl | tr -d '\n') && echo same)" "same"

echo "== payloads round-trip and match the entries' payload_hash"
ok=0; for n in {1..16}; do
  curl -s -H "Authorization: Bearer r-secret" $URL/payloads/p-$n > $OUT/got-p-$n
  want=$(sed -n ${n}p $OUT/expected.jsonl | python3 -I -c 'import json,sys; print(json.loads(sys.stdin.read())["payload_hash"])')
  [ "$want" = "$(sha256_b64url $OUT/got-p-$n)" ] && ok=$((ok+1))
done
check "16 payloads hash to their entries' payload_hash" "$ok" "16"
check "payload without a token is 401" "$(code $URL/payloads/p-1)" "401"

echo "== heartbeat"
waitfor "curl -sf $URL/heartbeat/latest -o $OUT/hb.json && [ \"\$(field $OUT/hb.json position)\" = 16 ]"
check "heartbeat names the head" "$(field $OUT/hb.json head_hash)" "$H16"
check "heartbeat signature verifies" "$(python3 -I $VERIFY_SIG $HUBKEY $OUT/hb.json)" "Verified OK"

echo "== restart"
stop; start
check "head survives restart" "$(head_hash)" "$H16"
check "position survives restart" "$(curl -s $URL/head | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["position"])')" "16"
mint 17 1 $H16 $OUT/r6
check "payload put" "$(put_payloads $OUT/r6)" "1"
check "submit after restart 201" "$(submit $OUT/r6 $OUT/r6.receipt)" "201"
curl -s $URL/aof > $OUT/aof2.jsonl
$P verify $OUT/aof2.jsonl > $OUT/verdicts2; check "upstream verify after restart: every line Sound" "$?" "0"
check "17 verdicts" "$(wc -l < $OUT/verdicts2 | tr -d ' ')" "17"
stop
echo "== done: $pass passed, $fail failed"
[ $fail = 0 ]
