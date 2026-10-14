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
it cannot account for. Heartbeats, distribution beyond the head, payloads
and webhooks are not built.

| Milestone | State |
| --- | --- |
| 1. Acceptor: per-line verification through the C ABI, conformance test | Done |
| 2. Append: writer thread, durability, crash recovery, receipts and refusals | Done |
| 3. Distribute and heartbeat: head, extension, whole file, lookup, heartbeats | Not started |
| 4. Payloads and webhooks: blob store behind an access policy, subscriptions | Not started |

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

## Usage

The Hub reads three environment variables.

| Variable | Required | Meaning |
| --- | --- | --- |
| `HUB_KEY` | yes | the Hub key: a P-256 private scalar as 64 hex characters |
| `HUB_DATA` | no | the data directory; defaults to `data` |
| `HUB_ADDR` | no | the listen address; defaults to `127.0.0.1:3717` |

A topic is a directory under the data directory holding a `topic.json` with
exactly these members. Creating topics over HTTP is not built.

```sh
mkdir -p data/demo
printf '{"topic_id":"demo","heartbeat_interval_seconds":60}' > data/demo/topic.json
HUB_KEY=$(openssl rand -hex 32) cargo run --release
```

The Hub prints its public key at startup. A key outside the P-256 scalar
range, a topic whose files do not reconcile, or a data directory with no
topics stops it there.

| Method and path | Body | Response |
| --- | --- | --- |
| `GET /topics/{t}/head` | | `{"head_hash":...,"position":...}` |
| `POST /topics/{t}/entries` | one or more Entry lines joined by `0x0a`, no terminator | `201` receipt, `409` refusal, `503` when the writer's queue is full |

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

Each topic directory holds `aof.jsonl`, the artifact, and `head`, a sidecar
with the committed count, head hash and byte length. On startup a whole
tail beyond the sidecar is re-judged and committed; a torn tail or a file
shorter than the sidecar stops the Hub.

## Repository layout

```
acceptor/      Haskell Cabal project over the eventchain libraries
  src/         the acceptor: one line judged by the verifier, verdicts coded for the ABI
  ffi/         the C-ABI shared library's exports; marshalling only
  include/     the C header; the ABI contract between the two languages
  tests/       the conformance test: every eventchain vector through the C ABI
src/
  main.rs      configuration and wiring
  acceptor.rs  safe Rust wrapper over the C ABI and the GHC runtime lifecycle
  topic.rs     the writer thread: judge, append, fsync, recover
  store.rs     the topic directory: settings, AOF, head sidecar; recover.rs is the reconciliation rule
  signer.rs    signed statements: receipts and refusals
  api.rs       the routes
build.rs       links the acceptor library built by cabal
docs/
  plan.md      the design of the v1 Hub
```

The acceptor pins the [eventchain](https://github.com/eventchain-protocol/eventchain)
repository by commit in `acceptor/cabal.project`, so a build is reproducible.

## License

Licensed under the [BSD 3-Clause License](LICENSE).
