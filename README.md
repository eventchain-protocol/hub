# hub

[![Status: work in progress](https://img.shields.io/badge/status-work%20in%20progress-orange)](#status)
[![License: BSD-3-Clause](https://img.shields.io/badge/license-BSD--3--Clause-blue)](LICENSE)

The reference Hub for the [EventChain protocol](https://eventchain-protocol.github.io/):
a role-gated payload store and AOF distributor, written in Rust, that accepts
already-signed Entries and appends them to per-topic append-only JSON Lines
files.

> [!WARNING]
> **Work in progress.** This crate is an early spike, not a usable Hub. It does
> not implement the protocol, its interfaces will change without notice, and it
> must not be used to hold real data.

## Status

The Hub appends. A Producer posts one signed line or a batch of them to a
topic; a writer thread judges every line through the acceptor, a Haskell
shared library that applies exactly the eventchain verifier's checks behind
a C ABI; the lines are written and fsynced whole or not at all; and the
Producer gets a signed receipt or a signed refusal. On restart the Hub
reconciles its head sidecar with the file and refuses to start on a topic
it cannot account for. It distributes the file as exact bytes: the whole
AOF, the extension after any Event ID, and a lookup of one line through an
embedded Event ID index. At each topic's interval it signs a heartbeat of
the committed head, keeps it in the topic's heartbeat list, serves that
list, and POSTs the heartbeat to the topic's heartbeat hosts. It stores
ciphertext Payloads it cannot read, by `payload_ref`, never replacing one,
and serves them only to bearer tokens the topic's access policy grants. It
POSTs each receipt to the topic's subscribers, retrying a failed delivery.
The operator creates topics over HTTP with an admin token.

| Milestone | State |
| --- | --- |
| 1. Acceptor: per-line verification through the C ABI, conformance test | Done |
| 2. Append: writer thread, durability, crash recovery, receipts and refusals | Done |
| 3. Distribute and heartbeat: head, extension, whole file, lookup, heartbeats | Done |
| 4. Payloads and webhooks: blob store behind an access policy, subscriptions | Done |

The design, its decisions and the milestone definitions are in
[`docs/plan.md`](docs/plan.md).

## Requirements

| Dependency | Version | Needed for |
| --- | --- | --- |
| Rust toolchain | stable, edition 2021 | the Hub |
| GHC | 9.12.4 | building `acceptor/` |
| cabal-install | 3.16 or later | building `acceptor/` |
| OpenSSL libcrypto | 3.2 or later | building and running |

libcrypto is a runtime dependency on every host, because the GHC runtime and
the eventchain signing library run inside the Hub process.

## Building

The Haskell acceptor library must be built before the Rust crate. The crate's
build script locates it under `acceptor/dist-newstyle/` and fails if it is
missing or built more than once.

```sh
(cd acceptor && cabal build)
cargo build --release
```

The acceptor's conformance test reads the vectors from the eventchain
checkout cabal makes under `acceptor/dist-newstyle/src/`:

```sh
(cd acceptor && cabal test)
```

### The end-to-end run

`tools/producer/` is a real client of the Hub: a Producer built on the
upstream eventchain Producer and Verifier at the acceptor's pinned commit.
Its driver starts a fresh Hub on a temporary data directory, creates a topic
over HTTP, mints entries under a random key, puts their payloads, submits
them from the live head in several rounds, including a stale head and a line
altered after signing, fetches the whole file, the extension and a lookup,
verifies the served file with the upstream verifier, checks every payload
against its entry's `payload_hash`, checks a heartbeat, restarts the Hub and
submits again. Every check prints `PASS` or `FAIL` and the exit status is
the count of failures. This run stands in for the tests the Hub does not
write.

```sh
(cd tools/producer && cabal build)
tools/producer/run.sh
```

## Usage

The Hub reads four environment variables.

| Variable | Required | Meaning |
| --- | --- | --- |
| `HUB_KEY` | yes | the Hub key: a P-256 private scalar as 64 hex characters |
| `HUB_ADMIN_TOKEN` | yes | the operator's bearer token, the only one that may create a topic; visible ASCII |
| `HUB_DATA` | no | the data directory; defaults to `data` |
| `HUB_ADDR` | no | the listen address; defaults to `127.0.0.1:3717` |

A topic is a directory under the data directory holding a `topic.json` with
exactly these members, every one required. `topic_id` names the directory.
`heartbeat_interval_seconds` is at least 1. `heartbeat_hosts` lists the
http or https URLs each heartbeat is POSTed to. `access` lists the bearer
tokens that reach the topic's Payloads, each with the role `read` (get every
Payload) or `write` (put and get every Payload, and subscribe); a token is
visible ASCII and appears once. Both lists may be empty, but they must be
there.

```sh
export HUB_KEY=$(openssl rand -hex 32) HUB_ADMIN_TOKEN=$(openssl rand -hex 16)
cargo run --release &
curl -H "Authorization: Bearer $HUB_ADMIN_TOKEN" --data-binary \
  '{"topic_id":"demo","heartbeat_interval_seconds":60,"heartbeat_hosts":[],"access":[{"token":"w-secret","role":"write"},{"token":"r-secret","role":"read"}]}' \
  http://127.0.0.1:3717/topics
```

A topic can also be made by hand, by writing the directory and its
`topic.json` before the Hub starts. The Hub prints its public key and each
topic it opened at startup. A key outside the P-256 scalar range, a missing
`HUB_ADMIN_TOKEN`, a topic whose files do not reconcile, a `topic.json` with
a missing or unknown member, or a `subscriptions.jsonl` it cannot read
stops it there. A data directory with no topics is fine.

| Method and path | Body or query | Response |
| --- | --- | --- |
| `POST /topics` | a `topic.json`, admin token | `201`; `409` if the topic's directory exists; `400` for settings the Hub would not open |
| `GET /topics/{t}/head` | | `{"head_hash":...,"position":...}` |
| `POST /topics/{t}/entries` | one or more Entry lines joined by `0x0a`, no terminator | `201` receipt, `409` refusal, `503` when the writer's queue is full |
| `GET /topics/{t}/entries?after={event_id}` | `after` required | the committed lines after that one, exact bytes as JSON Lines; `404` if the topic has no such line |
| `GET /topics/{t}/aof` | | the whole committed file, exact bytes |
| `GET /topics/{t}/lookup/{event_id}` | | `{"line":...,"position":...}`, the line as a JSON string; `404` if the topic has no such line |
| `GET /topics/{t}/heartbeat/latest` | | the latest heartbeat; `404` if none has been issued |
| `GET /topics/{t}/heartbeat` | `from`, `until` optional | a JSON array of the heartbeats with `from <= issued_at < until`; an omitted bound is open |
| `PUT /topics/{t}/payloads/{ref}` | ciphertext, `write` token | `201` stored; `200` if the ref holds these exact bytes; `409` if it holds others |
| `GET /topics/{t}/payloads/{ref}` | `read` or `write` token | the exact bytes, `application/octet-stream`; `404` if no such ref |
| `POST /topics/{t}/subscriptions` | one absolute http or https URL, `write` token | `201` subscribed; `200` if already subscribed |

The first line's `prev_hash` must be the current head; each later line's
must be the hash of the line before it. The Hub appends the body and one
`0x0a` whole or not at all, after every line passes the verifier's checks.

A receipt and a refusal are flat JSON objects of string members with `v`,
`kind`, `topic_id`, `submission_hash` (SHA-256 of the body), `issued_at`
and a `signature` by the Hub key over the JCS serialisation of the other
members. A receipt adds `position`, `head_hash` and `count`. A refusal adds
`reason`, with `line` for a verifier fault or `position` and `head_hash`
for `head_mismatch`. All hashes and the signature are base64url without
padding. The `reason` vocabulary is `enum ec_fault` in
`acceptor/include/eventchain_acceptor.h`, in snake case, plus
`head_mismatch`.

Every read serves the committed prefix only, up to the head the writer last
published. An Event ID is base64url without padding of the SHA-256 of a
line without its terminator; `after` may also be the genesis head, 32 zero
bytes, for the whole file. A malformed Event ID or timestamp, or an unknown
query parameter, is a `400`.

A token travels as `Authorization: Bearer <token>`. No token, a header in
any other form, or a token the topic does not grant is a `401`; a granted
token without the role the route needs, or any token but the admin token on
`POST /topics`, is a `403`. Tokens are compared in constant time.

A payload ref and a topic id are opaque to the Hub and are each one file
name: non-empty, printable ASCII, no `/`, no `..`, and not `.`; anything
else is a `400`. A topic id also cannot be `index`. The Hub stores a
Payload as exactly the bytes put and never replaces one; that the bytes are
the Payload an Entry names is the chain's to show, through its payload hash,
not the Hub's. A request body is limited to axum's default of 2 MiB.

After each accepted submission the Hub POSTs the receipt, the exact bytes
the Producer got, with `Content-Type: application/json`, to every URL
subscribed to the topic. A 2xx is delivered. Any other status, a redirect
included, no answer within 10 s, or a connection error is a failed attempt,
logged to stderr and retried after waits of 1, 4, 16 and 64 s in turn;
after the fifth attempt the delivery is given up. Deliveries are held in memory only: a restart drops
any not yet delivered, and receipts to one subscriber may arrive out of
order. A subscriber reconciles through `GET /topics/{t}/entries?after=`
from the last Event ID it holds, and orders by `position`. A refusal is not
delivered.

A heartbeat is a flat signed statement with `v`, `kind` (`"heartbeat"`),
`topic_id`, `position`, `head_hash`, `issued_at` and `signature`, as the
specification appendix states, signed like a receipt. One is issued at the
topic's interval once the topic has an entry, whether or not anything
arrived. It is written to the topic's heartbeat list before it is POSTed to
any host, with `Content-Type: application/json`; a POST that fails or does
not answer within the interval is logged to stderr and not retried.

Each topic directory holds `topic.json`; `aof.jsonl`, the artifact;
`head`, a sidecar with the committed count, head hash and byte length;
`heartbeats.jsonl`, every heartbeat issued for the topic, one per line;
`payloads/<ref>`, one file per Payload, the exact bytes put; and
`subscriptions.jsonl`, the subscribed URLs, one JSON string per line in the
URL's normal form, loaded on startup. On startup a whole tail beyond the
sidecar is re-judged and committed; a torn tail or a file shorter than the
sidecar stops the Hub, and so does a torn last line in `heartbeats.jsonl`
or `subscriptions.jsonl`.

`data/index/` holds the Event ID index, a redb store mapping each topic's
Event IDs to their position and place in the file. It is derived and never
authoritative: deleting it is safe, and on startup the Hub catches it up
from each AOF. An index ahead of a topic's sidecar, or naming a different
line than the file holds, stops the Hub. `index` cannot be a topic name.

## Repository layout

```
acceptor/      Haskell Cabal project over the eventchain libraries
  src/         the acceptor: one line judged by the verifier, verdicts coded for the ABI
  ffi/         the C-ABI shared library's exports; marshalling only
  include/     the C header; the ABI contract between the two languages
  cbits/       the C shim that starts and stops the GHC runtime for the Rust side
  tests/       the conformance test: every eventchain vector through the C ABI
src/
  main.rs      configuration and wiring
  acceptor.rs  safe Rust wrapper over the C ABI and the GHC runtime lifecycle
  topic.rs     the writer thread: judge, append, fsync, recover, index; topic creation; settings.rs the rules topic.json must meet
  store.rs     the topic directory: settings, AOF, head sidecar; recover.rs is the reconciliation rule, heartbeats.rs the heartbeat list,
               payloads.rs the payload blobs, subscriptions.rs the subscription file, segment.rs the file-name rule
  index.rs     the Event ID index under data/index/: extended by the writer, caught up from the AOF on open
  heartbeat.rs the heartbeat scheduler and publisher; window.rs is the from/until range rule
  access.rs    the AccessPolicy trait, its per-topic bearer-token implementation, and the operator token
  webhook.rs   receipt delivery to subscribers, with retry, in memory
  signer.rs    signed statements: heartbeats, receipts and refusals
  api.rs       the head and submission routes; topics.rs creation, distribute.rs the extension, file and lookup,
               heartbeat.rs the list, payloads.rs the blobs, subscriptions.rs subscribing
build.rs       links the acceptor library built by cabal
tools/
  producer/    the end-to-end run: a Producer on the upstream libraries, its driver, and the statement signature check
docs/
  plan.md      the design of the v1 Hub
```

The acceptor pins the [eventchain](https://github.com/eventchain-protocol/eventchain)
repository by commit in `acceptor/cabal.project`, so a build is reproducible.

## License

Licensed under the [BSD 3-Clause License](LICENSE).
