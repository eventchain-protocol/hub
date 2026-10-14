# Plan: the reference Hub (v1)

A decision document. It states what the Hub is, what it reuses, where its
boundaries sit and which open protocol encodings it fixes. It contains no
implementation. Review it for whether these are the right decisions.

Protocol source of truth: the white paper, in particular *How EventChain
Works* §Append Protocol, §Hub Heartbeat and the Specification appendix.
Vocabulary: `eventchain/CONTEXT.md`. Terms there are used here exactly.

## What the Hub is

The Hub is the role-gated payload store and AOF distributor. It holds one
key, for heartbeats and signed receipts and refusals, and never signs an
Entry. It receives already-signed Entries and appends them. It never sees a
plaintext Payload.

Per topic it does five things:

| Duty | Rule it enforces |
| --- | --- |
| Append | Accept a submission only if the line passes exactly the per-line checks a verifier applies and its `prev_hash` is the current head |
| Answer | Every submission gets a signed receipt or a signed refusal |
| Distribute | Serve the head, the extension since any Event ID, and the whole AOF, as exact bytes |
| Heartbeat | Sign a statement of (position, head hash, time) at the topic's interval, even when nothing arrived |
| Store | Put and get ciphertext Payloads by `payload_ref` under an access policy; answer Event ID lookups |

## Decision 1: the acceptor is the Haskell verifier, called through a C ABI

The Hub does not re-implement the per-line checks. It calls
`eventchain-verify`, which already performs them: strict framing, the
duplicate-member and surrogate rules, JCS, continuity, attribution and the
Mint envelope checks. "The Hub applies exactly the checks a verifier applies
to one line" becomes literally true, and the committed vectors in the
eventchain repo are the acceptor's conformance suite.

The Hub itself is written in Rust. The seam between them is a shared
library with a C ABI, built from a Cabal `foreign-library` stanza. Nothing
but bytes and small integers cross it.

**Where the Haskell package lives: in this repo**, under `acceptor/`, as a
Cabal project that pins the eventchain repo by commit. The ABI is the Hub's
contract, so it is versioned with the Hub. The eventchain repo is untouched.
It is a consumer of `eventchain-verify` and `eventchain-crypto`, which the
vocabulary doc already names as the Hub's relationship to those packages.

**What the ABI exports**, by purpose rather than signature:

- *Runtime lifecycle.* Start and stop the GHC runtime once per process.
- *Accept one line.* Input: the topic's entry count, the current head hash,
  the submitted line bytes. Output: one of the verifier's four verdicts
  (Sound, Malformed, Unsound, Undecided) with its fault vocabulary flattened
  to an enum, and on Sound the line hash, which is the Event ID. A count of
  zero means the genesis rule applies.
- *Load the Hub key.* Key bytes in once at startup, an opaque handle out.
  The private key never comes back across the boundary.
- *Sign a statement.* A heartbeat, receipt or refusal is a flat JSON object
  of string members. The Haskell side canonicalises it with JCS and signs it
  with the handle; the signature comes back. Canonicalisation stays on the
  Haskell side so that there is one JCS implementation in the Hub, and it is
  one that is already gated against the aeson RFC 8785 oracle.

**What the ABI does not do.** It verifies one line per call. The Hub never
hands it a file. Bulk verification is the verifier's own job, from the file.

**Limits this carries.** The GHC runtime runs inside the Hub process. GHC
9.12 and libcrypto 3.2+ are build dependencies, and libcrypto is a runtime
dependency on every host. The ABI is a breaking-change contract: a verdict
enum and a header file, both pinned in `acceptor/`. Under this repo's rules
a change there is a hard break on both sides, never a shim.

## Decision 2: one writer thread per topic owns the file and the head

The paper says the Hub appends on a single thread per topic. The Hub makes
that the structure, not a lock discipline:

```
HTTP handler ──submission──▶ topic writer thread ──receipt/refusal──▶ handler
                                   │
                        owns: AOF file handle, head (count, hash)
                        does: compare head, call acceptor, append, fsync,
                              write head sidecar, index Event ID
```

The handler is async; the writer is a plain OS thread that blocks on the
acceptor call (about 50 µs) and the fsync. Submissions arrive over a bounded
channel, so back-pressure reaches the HTTP layer as a 503 rather than an
unbounded queue. The compare-and-swap on the expected head happens on the
writer thread, where there is nothing to race.

**Limit.** A thread per topic is right for tens to low thousands of topics.
Beyond that a writer pool with topic affinity replaces it; that is a later
change inside one module, and it is not built now.

## Decision 3: durability rule and crash recovery

A receipt is issued only after the line's bytes are fsynced to the AOF and
the head sidecar has been atomically replaced. Order matters:

1. Append the exact submitted bytes plus `0x0a` to the AOF; fsync.
2. Write (count, head hash) to a temporary file; fsync; rename over `head`.
3. Index the Event ID; reply.

On startup the writer reads `head`, measures the AOF and reconciles. The
only crash window is between steps 1 and 2, which leaves at most one line
beyond the sidecar. That line is re-run through the acceptor against the
sidecar's head and, if Sound, the sidecar is advanced. Anything else is a
hard error: the operator is told the topic is inconsistent, and the Hub does
not guess.

The Hub trusts its own sidecar the way a holder trusts its own Checkpoint.
It does not re-verify the whole AOF on every start. An operator who wants
that runs the verifier on the file.

Group commit is not built. The protocol commits one Entry per race window
per topic, so there is never a batch to group.

## Decision 4: storage is files, plus one embedded index

```
data/
  <topic>/
    topic.json        settings: topic id, heartbeat interval, hub key id
    aof.jsonl         the artifact; append-only; exact bytes
    head              count and head hash; replaced atomically
    heartbeats.jsonl  every heartbeat ever issued, append-only
    payloads/<ref>    ciphertext blobs
  index/              embedded key-value store: Event ID → (topic, position, byte offset)
```

The AOF is the artifact and no database holds it. The index is derived and
rebuildable from the files; it is never authoritative. It lives in `redb`,
a pure-Rust embedded store, so startup is O(1) and memory stays bounded
however many Entries exist. An in-memory map would be simpler and was
rejected because it grows without bound and is lost on every restart.

## Decision 5: the HTTP surface, and the three encodings it fixes

The paper leaves the submission format, the receipt and refusal encodings
open and requires a signed refusal. The Hub fixes v1 of each. Each is a
proposed paper amendment, recorded as an ADR here.

| Method and path | Body or query | Response |
| --- | --- | --- |
| `POST /topics` | topic settings | 201 |
| `POST /topics/{t}/entries` | the line bytes; `Expected-Head` header | 201 receipt, or 409 refusal with the current head |
| `GET /topics/{t}/head` | | position and head hash |
| `GET /topics/{t}/entries?after={event_id}` | | JSON Lines tail, exact bytes |
| `GET /topics/{t}/aof` | | the whole file, exact bytes |
| `GET /topics/{t}/lookup/{event_id}` | | position and the line |
| `GET /topics/{t}/heartbeat/latest`, `/heartbeat`, `/heartbeat?from=&until=` | | as the specification appendix states |
| `PUT`, `GET /topics/{t}/payloads/{ref}` | ciphertext | gated by the access policy |
| `POST /topics/{t}/subscriptions` | a URL | 201; each commit is POSTed to it |

**Submission.** The body is the line's exact bytes, without a terminator.
The Hub adds the `0x0a`. The submission hash is SHA-256 of those bytes, which
on acceptance is the Event ID. One hash, two roles.

**Receipt and refusal.** The same shape as a heartbeat, so a verifier that
checks one checks all three: a flat object of string members, `v` = `"1"`,
`kind` = `"receipt"` or `"refusal"`, `topic_id`, `submission_hash`,
`issued_at`, and `signature` over the JCS of the rest. A receipt adds
`position`. A refusal adds `reason`, drawn from a closed vocabulary that is
the verifier's fault names plus `head_mismatch`, and on `head_mismatch` the
current `position` and `head_hash` so the submitter can fetch the extension
without a second round trip.

**Freshness.** A rejected submission is never retried by the Hub and the
submitter's signature over the old head is never reused; the submitter
fetches the extension, verifies it, signs again. That is the paper's rule
and the Hub does nothing to soften it.

**Topic creation.** The binding between a topic and its first Entry is open
in the paper and the Hub does not invent one. `POST /topics` records
settings only.

## Decision 6: the Hub key and the heartbeat list

The Hub key is ECDSA-P256. It is loaded once, from a file, into the Haskell
side; the Rust side holds a `Signer` trait with that one implementation. A
KMS or HSM implementation is a second implementation of the trait and
touches nothing else. Key custody is the operator's problem and the
reference says so.

The heartbeat scheduler ticks per topic at the topic's interval, asks the
writer thread for (count, head), signs the statement and appends it to
`heartbeats.jsonl`. The Hub serves the list endpoints itself, and when a
keeper URL is configured it also POSTs each heartbeat there. Serving the
list from the Hub is a documented weakening: it lets a single holder detect
staleness, but a Hub that is its own keeper can show two holders two lists.
The keeper role is the foundation's, and the reference Hub is built so the
publish step can point elsewhere.

## Decision 7: access policy is a trait with one deliberately small implementation

The paper makes payload access application policy. The reference ships an
`AccessPolicy` trait and one implementation: per-topic bearer tokens mapped
to a role, where a role may read all of a topic's Payloads or none. That is
enough to show the Hub enforcing policy without decryption keys and not
enough to be mistaken for an authorisation system.

## What is out of scope and why

- Registries: a registry is a topic like any other, so the Hub needs no
  code for it.
- OpenTimestamps anchoring and witness cosigning: optional tiers above the
  Hub, separate services.
- Payload encryption and salt handling: the Producer's, before bytes reach
  the Hub.
- Identity binding and attribution outcomes: the verifier's and the
  application's.

## Repository layout

```
acceptor/      Haskell. Cabal project; foreign-library; the C header; vector-driven conformance test
src/
  main.rs      configuration and wiring only
  acceptor/    safe Rust wrapper over the C ABI, runtime lifecycle
  topic/       the writer thread: compare, accept, append, fsync, recover
  store/       AOF file, head sidecar, heartbeat file, payload blobs, index
  api/         routes, one file per resource
  heartbeat/   scheduler and publisher
  signer/      the trait and the Haskell-backed implementation
  webhook/     subscriptions and delivery with retry
docs/
  plan.md      this document
  adr/         one record per fixed encoding and per boundary decision
```

Each directory is one concept. No file in `src/` carries two of them.

## Tests

Under this repo's rules, tests exist only where a pure function branches and
a bug would be silent. Two places qualify:

- The acceptor's conformance test: every committed eventchain vector pushed
  through the C ABI one line at a time, asserting the verdict the verifier
  gives. Bytes in, enum out, no mock.
- Crash recovery: the reconciliation rule over (sidecar, file length,
  trailing line) is a pure decision with four outcomes.

Everything else is exercised by running the Hub against a Producer and
verifying the resulting file with the verifier, which is what the file is
for.

## Build

`acceptor/` builds with Cabal into a shared library and a header. The Rust
crate's build script links it and libcrypto. One `just build` runs both.
Prebuilt acceptor artifacts per platform are a later convenience, not a
v1 requirement.

## Milestones

1. **Acceptor.** The Haskell package, the ABI, the conformance test. Done
   when every vector gives the verifier's verdict through the C ABI.
2. **Append.** Writer thread, durability rule, recovery, receipt and
   refusal. Done when a Producer can append through HTTP and the verifier
   accepts the file.
3. **Distribute and heartbeat.** Head, extension, whole file, lookup,
   scheduler, list endpoints.
4. **Payloads and webhooks.** Blob store behind the policy trait,
   subscriptions with retry.
