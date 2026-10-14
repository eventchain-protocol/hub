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

The acceptor is built: a Haskell shared library that judges one line as the
eventchain verifier would, loads the Hub key and signs, behind a C ABI. Its
conformance test runs every committed eventchain vector through the exported
C functions and checks each verdict against the verifier's. The Hub itself is
still a spike: a web server that signs whatever JSON is posted to `/`.

| Milestone | State |
| --- | --- |
| Rust-to-Haskell seam (C ABI, runtime lifecycle, signing) | Spike working |
| 1. Acceptor: per-line verification through the C ABI, conformance test | Done |
| 2. Append: writer thread, durability, crash recovery, receipts and refusals | Not started |
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

The spike reads two environment variables.

| Variable | Required | Meaning |
| --- | --- | --- |
| `HUB_KEY` | yes | the signing key: a P-256 private scalar as 64 hex characters |
| `HUB_ADDR` | no | the listen address; defaults to `127.0.0.1:3717` |

Start it with a throwaway key:

```sh
HUB_KEY=$(openssl rand -hex 32) cargo run --release
```

Post any JSON document to `/`:

```sh
curl -s -X POST http://127.0.0.1:3717/ -d '{"hello":"world"}'
```

The response holds an ECDSA-P256 signature over the exact request bytes and
the compressed public key, both base64url without padding:

```json
{"public_key":"...","signature":"..."}
```

A body that is not JSON gets `400 Bad Request`. A key outside the P-256 scalar
range stops the Hub at startup.

## Repository layout

```
acceptor/      Haskell Cabal project over the eventchain libraries
  src/         the acceptor: one line judged by the verifier, verdicts coded for the ABI
  ffi/         the C-ABI shared library's exports; marshalling only
  include/     the C header; the ABI contract between the two languages
  tests/       the conformance test: every eventchain vector through the C ABI
src/
  main.rs      the spike web server
  acceptor.rs  safe Rust wrapper over the C ABI and the GHC runtime lifecycle
build.rs       links the acceptor library built by cabal
docs/
  plan.md      the design of the v1 Hub
```

The acceptor pins the [eventchain](https://github.com/eventchain-protocol/eventchain)
repository by commit in `acceptor/cabal.project`, so a build is reproducible.

## License

Licensed under the [BSD 3-Clause License](LICENSE).
