# JEV Calendar Rust backend

A native Rust replacement for the current book.jaack.me private API and public gateway. One compiled binary runs in two modes. The booking frontend, SQLite schema, OAuth URLs, encrypted credentials, capability links, host/viewer timezone rules, workload policy and JAACK OS integration remain compatible.

Status on 6 October 2026: production is running Rust in both private API and public gateway modes after Jaack explicitly authorized the full switch. Release `20261006T115852Z-d242ac812051` serves the original ports, database and URLs with frontend v18. Both listeners and public-domain health verified the Rust marker and ready calendar previews. Browser readback confirmed automatic selection of 7 October, the earliest eligible date, and subsequent day changes with live slots and no error. All eight existing booking rows match the consistent pre-deployment backup. No live events or invitations were created by this work. Fresh booking conflicts remain covered by synthetic provider tests.

The exact previous LaunchAgent plists and consistent database backup are retained in `.local/cutover-20261005T213517Z-b4f5df9e9d55`. Node is retained for rollback and is not running either production service. The proposed 48-hour observation period has not elapsed. Activation and readback evidence are in `evidence/cutover*.json`, `evidence/cutover.log` and `evidence/live-functional-readback.json`.

Source maintenance on 6 October removed an unused direct dependency, expanded the advisory and persistence code, and fixed release baseline checks after production status documentation changes. Release tooling now records the previous backend, so failed upgrades and rollback can restore an earlier Rust release as well as Node. That maintenance pass did not restart the then-active initial release.

Earlier on 6 October, the public month calendar failed when Google Routes returned no usable duration. Route decoding now rejects missing/invalid durations with a controlled provider error. Month scanning withholds the affected day, reports partial availability and continues checking other dates, including other days with in-person events. Explicit zero/fractional durations remain valid. The regression failed against the prior binary and passed with the fix. This behavior intentionally differs from the Node oracle, which silently converted missing durations to zero travel. Public browser readback verified 16 selectable October dates and live slots at 15:35 and 16:05 Europe/Rome on 9 October. Both listeners and public-domain health returned Rust; all eight existing booking rows remained identical. The previous Rust release is retained in the latest cutover directory, alongside the original Node rollback material.

## Earliest dates and background advice

The shared-snapshot release was verified in public Chrome after realistic idle periods. A date click about 84 seconds after initial slots appeared took 17.2 ms to slot DOM insertion and 26.6 ms to two subsequent animation frames; its background recheck completed in about 341 ms. After the original preview's full source-age bound expired, another click took 169.7 ms to DOM insertion and 181.4 ms through the two frames. A later expired day took 191.6 ms, while a recent preview took 12.1 ms during revalidation. The earlier cold-path samples were 1.935 and 6.246 seconds with the same DOM/animation-frame method. These are individual desktop Chrome measurements, not mobile or percentile claims.

Initial navigation still took 1.865 seconds to a paint opportunity for populated slots in this Chrome run, including a 1.278-second month request. The optimized isolated real-calendar preflight measured 226.1 ms for the month and 12.8 ms for a day. Live loopback timing varied from that preflight, and public day API samples were 140-327 ms, so the initial-load target is not met consistently. Production first sync took 2.578 seconds before the private listener became available. Browser source-age resets confirm continuing independent refreshes; no new Jev advice was requested by these visits. Earlier Chrome timing evidence is retained in `evidence/shared-v17-readback.json` and `evidence/shared-browser.json`; the isolated preflight is in `evidence/shared-final-preflight.json`.

Final frontend v18 is live in release `20261006T115852Z-d242ac812051`. The selected day automatically rechecks while the calendar is visible and when returning to a visible tab. Entering booking details stops preview timers and ignores an earlier in-flight response; confirmation still checks fresh Google conflicts. Thirteen frontend regressions cover these behaviors. Chrome disconnected during the final verification, so the final release was checked in Codex's in-app browser. Hard-expired day changes took 128.8 and 268.3 ms through the two-frame measurement; recently refreshed days took 13.6 and 13.0 ms. A selected-day check occurred automatically after about 52 seconds idle and took 122.6 ms. No availability reads occurred while the untouched details form remained open, and going back checked the date again. Final public month API read was 280.9 ms and public day reads were 97.7-342.3 ms; the earlier initial-page limitation remains explicit rather than inferred from these API timings. All eight bookings, saved advice, notice and spacing remained unchanged. Final evidence: `evidence/shared-readback.json` and `evidence/shared-final-browser.json`.

The first attempted shared-snapshot activation was restored to the previous working Rust release when browser readback found unavailable previews. Google rejected the broad free/busy request with `timeRangeTooLong`; thirty-day windowing was verified against real Google reads on a separate database copy, added to the fake provider, and then redeployed. Readiness now checks the snapshot as well as process health. Both attempted release backups and the previous working launch configuration are retained; no old database was restored.

Earlier frontend v16 measurements: On 6 October, four public-browser day changes took 1.328-2.131 seconds before reuse and 7.4-10.7 milliseconds after deployment (median 1.627 seconds versus 8.35 milliseconds). Timing runs from the captured date click to the next animation frame after slot DOM insertion. The same dates and measurement method were used; these are four samples per release, not a population percentile. All four fresh-preview switches made zero network requests. After expiry, one verified fresh read took 5.589 seconds, so Google Calendar latency was still visible when the browser preview expired. The month request with seventeen prepared days was 1.693 seconds and 10,155 bytes. Booking/advice state stayed unchanged. Evidence: `evidence/day-switch-readback.json`.

Before the day-switch reuse change, a live read-only sample measured October month availability at 1.556 seconds, compared with the earlier 14.292-second sample; these are individual timings, not latency percentiles. That earlier readback had seventeen eligible dates after routing removal. The saved advice timestamp stayed unchanged across verification visits and a deployment restart. Its next six-hour deadline is 18:25 Europe/Rome on 6 October. The six-hour/restart behavior has automated coverage; a complete six-hour production cycle has not elapsed. Current readback is retained in `evidence/background-readback.json`.

The booking page opens the earliest available date, preserving an explicitly selected date during a refresh. The private service warms a shared calendar snapshot at startup, then refreshes it every sixty seconds independently of website visits. All connected calendars, including hidden, shared and free/busy-only calendars, must return complete results before a new snapshot replaces the previous one. The rolling window includes the ninety-day booking horizon and full weekly workload context. Google free/busy reads use thirty-day windows, avoiding the observed `timeRangeTooLong` rejection; calendar discovery is reused across windows. Provider failure keeps the last complete snapshot only up to its two-minute hard age limit. Account versions, booking destination and host timezone identify a snapshot; configuration and local booking changes invalidate it and wake the worker. A generation check rejects refreshes overtaken by a mutation. Current rules, notice, spacing and feedback still apply at read time.

Month scans trim the shared data to their required workload window and score only the seven eligible days used by the recommendation, avoiding unnecessary work over the full horizon. Fixed date, month, workload and virtual-location matchers compile once per process. Workload scoring clips each event to the scored day before matching its project and type, avoiding repeated classification of events from other days. These changes preserve the same scheduling policy and add no further result cache. The month response includes `firstDateAvailability` and `availabilityByDate`, exposing only public slots and source-age metadata. The browser reuses recent results keyed by date, duration and viewer timezone, shows them immediately after the sixty-second freshness threshold, and rechecks them in the background. While the calendar is visible, its selected day also rechecks automatically before the hard deadline; returning to a visible tab rechecks again. Entering booking details stops preview timers and ignores any earlier recheck response so a stale-offer timeout cannot interrupt the form. Going back to the calendar checks the selected day again. Request time and source age count toward the two-minute hard bound; day changes never extend it. Disappearing selections are withdrawn, valid selections survive revalidation, and late replies cannot overwrite another date. A timer withdraws expired previews even if a recheck fails. Server invalidation clears all browser previews. Booking creation and changes bypass the snapshot and verify current Google conflicts before reserving a slot. Jev workload advice remains on its separate six-hour schedule.

Public booking JavaScript and the booking/privacy stylesheet use the first sixteen hex digits of their SHA-256 contents in the asset URL. The gateway grants a one-year immutable cache lifetime only when that version matches the private asset's complete digest ETag. Unversioned, numeric-version and mismatched assets keep the existing five-minute lifetime; failed asset responses and all page/API responses use no-store. GET and HEAD have matching cache metadata. When changing an asset, update every referencing page to its new content digest; the frontend regression checks the actual release overlay and reference fallback. This reduces repeat-visit asset downloads without caching availability or introducing a persistent slot cache.

The private listener starts after its initial calendar warmup attempt; release health checks require ready calendar previews and allow sixty seconds for startup. A failed initial sync leaves preview reads unavailable until a complete background retry succeeds. The snapshot is held in private-process memory and is rebuilt after a restart; it adds no database schema, persistent event cache or public event details. Explicit API dates outside the booking page's horizon retain their existing fresh-read behavior.

Google Routes and per-visit Jev travel advice are removed. Configured minimum spacing between meetings remains enforced against calendar conflicts. The private service worker refreshes aggregate Jev workload advice every six hours, using the persisted refresh timestamp across restarts; public visits and settings/timezone/rule saves do not run Jev. An explicit private manual workload refresh remains available. Until a changed policy is refreshed, the configured notice floor and deterministic workload rules apply. The saved eight-hour notice and twenty-minute buffer are unchanged by this implementation.

Current browser overrides live in `public/`; the immutable Node oracle remains in `reference/`. Release packaging overlays these files onto the captured assets and checksums the result. Privacy and private-dashboard text reflect the background cadence and removed routing.

## Layout

- `src/`: native HTTP, persistence, cryptography, scheduling, workload, provider and booking implementations.
- `reference/`: generated SHA-256 identified snapshot of the existing Node source and unchanged browser assets, including its pre-existing local edits. No production secrets or databases.
- `public/`: current booking, privacy and private-dashboard overrides.
- `fixtures/`: synthetic default rules, exact bounded Jev questions and the compressed immutable Node baseline.
- `tests/`: cross-language comparisons, preserved HTTP assertions, local fake-provider failure injection, gateway and release rehearsals.
- `scripts/release.py`: checksum packaging, read-only preflight, consistent SQLite backup, guarded LaunchAgent activation and rollback.
- `evidence/`: retained build/check/test logs, synthetic comparison results, screenshots and local benchmark.
- `dist/`: packaged binary and checksummed frontend assets. Release identity is recorded in `evidence/release-path.txt`.

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
node --test tests/frontend.mjs
cargo build --release --locked -j 2
python3 tests/release.py
```

Node is used only as a test oracle and retained rollback implementation. The Rust services execute no Node process. The preserved Node baseline passed all 80 tests and syntax checks. Rust validation passed four native unit tests, 5,932 deterministic comparisons across nine timezones/DST dates, six preserved HTTP compatibility tests, fifteen fake-provider integration tests, one gateway suite, nine release/reference rehearsal tests and fourteen frontend regressions. Tests use disposable SQLite databases, synthetic credentials and loopback providers.

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

The package contains a release binary, captured assets with current frontend overrides, lockfile, toolchain record, launch wrappers and SHA-256 manifest. Wrappers read the existing private .env in its existing checkout; secrets are not copied into the package. The Cloudflare Tunnel, Tailscale owner origin, OAuth redirect and ports remain stable.

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
