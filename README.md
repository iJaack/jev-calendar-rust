# JEV Calendar Rust backend

A native Rust replacement for the current book.jaack.me private API and public gateway. One compiled binary runs in two modes. The booking frontend, SQLite schema, OAuth URLs, encrypted credentials, capability links, host/viewer timezone rules, workload policy and JAACK OS integration remain compatible.

Status on 6 October 2026: production is running Rust in both private API and public gateway modes after Jaack explicitly authorized the full switch. Release `20261006T094423Z-4af31980ad89` serves the original ports, database, frontend and URLs. Independent loopback and public-domain readback verified the Rust marker, Google connection state, protected routes and authenticated JAACK OS reads. A bounded live availability request returned 200 with no offered slots for 7 October under the current policy. A live booking lifecycle remains unverified; no live events or invitations were created by this work.

The exact previous LaunchAgent plists and consistent database backup are retained in `.local/cutover-20261005T213517Z-b4f5df9e9d55`. Node is retained for rollback and is not running either production service. The proposed 48-hour observation period has not elapsed. Activation and readback evidence are in `evidence/cutover*.json`, `evidence/cutover.log` and `evidence/live-functional-readback.json`.

Source maintenance on 6 October removed an unused direct dependency, expanded the advisory and persistence code, and fixed release baseline checks after production status documentation changes. Release tooling now records the previous backend, so failed upgrades and rollback can restore an earlier Rust release as well as Node. That maintenance pass did not restart the then-active initial release.

On 6 October, the public month calendar failed when Google Routes returned no usable duration. Route decoding now rejects missing/invalid durations with a controlled provider error. Month scanning withholds the affected day, reports partial availability and continues checking other dates, including other days with in-person events. Explicit zero/fractional durations remain valid. The regression failed against the prior binary and passed with the fix. This behavior intentionally differs from the Node oracle, which silently converted missing durations to zero travel. Public browser readback verified 16 selectable October dates and live slots at 15:35 and 16:05 Europe/Rome on 9 October. Both listeners and public-domain health returned Rust; all eight existing booking rows remained identical. The previous Rust release is retained in the latest cutover directory, alongside the original Node rollback material.

## Layout

- `src/`: native HTTP, persistence, cryptography, scheduling, workload, provider and booking implementations.
- `reference/`: generated SHA-256 identified snapshot of the existing Node source and unchanged browser assets, including its pre-existing local edits. No production secrets or databases.
- `fixtures/`: synthetic default rules, exact bounded Jev questions and the compressed immutable Node baseline.
- `tests/`: cross-language comparisons, preserved HTTP assertions, local fake-provider failure injection, gateway and release rehearsals.
- `scripts/release.py`: checksum packaging, read-only preflight, consistent SQLite backup, guarded LaunchAgent activation and rollback.
- `evidence/`: retained build/check/test logs, synthetic comparison results, screenshots and local benchmark.
- `dist/`: packaged binary and unchanged assets. Release identity is recorded in `evidence/release-path.txt`.

## Build and validate

Rust 1.93.1 is pinned. Cargo.lock records the exact resolved dependencies. Keep compilation to two jobs on the shared Mac. Python 3.12 or later restores the captured Node test oracle on a fresh checkout and rejects modified reference files. Build outputs, release packages, credentials, SQLite databases and private backups are excluded from Git; retained raw evidence stays local.

```sh
python3 scripts/restore_reference.py
cargo build --locked -j 2
cargo fmt --all --check
cargo clippy --locked --all-targets -j 2 -- -D warnings
cargo test --locked -j 2
node tests/differential.mjs
node tests/http-compat.mjs
node --test --test-concurrency=1 tests/providers.mjs
node --test tests/gateway.mjs
cargo build --release --locked -j 2
python3 tests/release.py
```

Node is used only as a test oracle and retained rollback implementation. The Rust services execute no Node process. The preserved Node baseline passed all 80 tests and syntax checks. Rust validation passed three native unit tests, 5,920 deterministic comparisons across nine timezones/DST dates, six preserved HTTP compatibility tests, eleven fake-provider integration tests, one gateway suite and six release/reference rehearsal tests. Tests use disposable SQLite databases, synthetic credentials and loopback providers.

The fake-provider suite verifies hidden/shared calendars, pagination, 50-calendar free/busy batching, incomplete-provider fail-closed behavior, single-use OAuth and pinned owner, stale refresh rejection, original event/account/Meet preservation, ETag changes, attendee updates, creation/cancellation lost-response recovery, conflicting guests, local confirmation failure, restart recovery and SIGTERM drain. Gateway coverage checks route/method isolation, origin/host/header/body validation, credential stripping and rate limits.

## Run an isolated demo

Choose an unused loopback port outside 3030/3031. Default demo mode uses synthetic calendars and makes no provider requests.

```sh
PORT=13030 DATABASE_PATH=.local/demo.sqlite \
APP_ENCRYPTION_KEY=synthetic-demo-key DEMO_MODE=true \
APP_BASE_URL=http://127.0.0.1:13030 JEV_ASSET_ROOT=reference/public \
target/release/jev-calendar --private
```

The existing environment names are preserved: PORT, PUBLIC_PORT, PUBLIC_HOST, INTERNAL_ORIGIN, APP_BASE_URL, DATABASE_PATH, APP_ENCRYPTION_KEY, DEMO_MODE, NODE_ENV, GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET, GOOGLE_ROUTES_API_KEY, TYPESAFE_API_KEY, JEV_JAACK_OS_READ_TOKEN and JEV_JAACK_OS_WRITE_TOKEN. JEV_ASSET_ROOT selects immutable assets. Production ports and the production data directory require JEV_ALLOW_LIVE_STATE=1. Local fake provider overrides require explicit test mode, HTTP loopback and no live-state permission.

## Intentional behavior difference

Google creation now has a deterministic event identity persisted before dispatch. A lost creation response, restart or local confirmation failure leaves a pending reservation and reads Google before retrying the same identity. This avoids blind duplicate dispatch; it does not promise exactly-once delivery. Changed payloads under the same request key are rejected. Legacy ambiguous creations without a stable event identity require reconciliation before replay.

All provider awaits occur outside SQLite transactions. Account changes and booking mutations share one asynchronous lock. Token refresh has version checks before and after dispatch. Public mode never opens the booking database.

## Release and rollback

```sh
python3 scripts/release.py package
python3 scripts/release.py inspect --database /absolute/path/to/database.sqlite
python3 scripts/release.py backup --database /absolute/path/to/database.sqlite --destination /new/private/backup.sqlite
```

The package contains a release binary, unchanged assets, lockfile, toolchain record, launch wrappers and SHA-256 manifest. Wrappers read the existing private .env in its existing checkout; secrets are not copied into the package. The Cloudflare Tunnel, Tailscale owner origin, OAuth redirect and ports remain stable.

After production cutover authorization, use the verified release path and actual production database:

```sh
python3 scripts/release.py switch --live --release /absolute/path/to/release --database /absolute/path/to/database.sqlite
```

Activation verifies baseline/release hashes and unresolved-operation guards, saves exact existing LaunchAgent plists, stops the public gateway before the private service, waits for process exit, rechecks intents, takes a consistent backup, changes the existing two LaunchAgents and checks both backends. It never starts Node and Rust together as production writers. Failed activation restores the retained launch configuration only when current state passes reconciliation guards. If draining or reconciliation fails, services stay blocked for inspection rather than forcing a kill or replay.

```sh
python3 scripts/release.py rollback --live --destination /absolute/path/to/cutover-directory --database /absolute/path/to/database.sqlite
```

Rollback restores the retained Node or Rust backend using the current compatible database, including bookings made after cutover. The initial Node cutover directory predates backend metadata and remains supported. It never overwrites current state with the pre-cutover backup. Pending bookings, pending changes and uncertain/pending JAACK OS journal entries block switching until reconciled. An authorized live acceptance and the proposed 48-hour observation period remain separate release outcomes. Retain Node and rollback materials until those outcomes are recorded.

## Browser and performance evidence

The unchanged pages were exercised in the Codex in-app browser at its initial 1265 x 712 viewport, 1024 x 768 and 390 x 844. Demo create, reschedule, cancellation and invalid capability error were verified, with no observed console warnings/errors. Screenshots are in evidence/ui-*.jpg. Owner OAuth/provider behavior is covered with fake providers; no physical device or owner sign-in was requested.

A single local sequential demo benchmark after warmup measured 100 availability requests per backend: Node p50 7.28 ms, p95 10.32 ms and RSS 253 MiB; Rust release p50 2.31 ms, p95 2.85 ms and RSS 14 MiB. Both made zero provider calls. These figures describe this synthetic run, not live end-user performance. See evidence/performance.json. Google Routes quotas and external request latency remain relevant.
