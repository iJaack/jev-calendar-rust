# Preserved backend contract

The authoritative oracle is reference/, identified by evidence/baseline-manifest.json. HTTP tests change launchers and import paths only, preserving the original assertions. Browser assets are copied unchanged into the release.

| Surface | Methods | Boundary |
| --- | --- | --- |
| /, /manage, /privacy, /assets/* | GET, HEAD | Public static files; canonical path containment |
| /health, /api/config | GET, HEAD | Public bounded status, no account identities or credentials |
| /api/availability, /api/available-dates | POST | Public JSON, host/origin/header checks and rate limiting |
| /api/book | POST | Public named guest/email, fresh availability, durable reservation and idempotency |
| /api/booking/view, /api/booking/slots, /api/booking/change | POST | Signed guest capability, expiry, revision and provider reconciliation |
| /owner | GET, HEAD | Private dashboard; public shortcut redirects to configured private origin |
| /auth/google, /auth/google/callback | GET | Private OAuth cookie, single-use state, owner/session binding |
| /api/google/accounts | GET, PUT, DELETE | Private owner session; mutations also require same-origin owner intent |
| /api/google/connect | POST | Private owner session and same-origin intent |
| /api/settings | GET, PUT | Private owner session; signed address selection and bounded settings |
| /api/timezone | PUT | Private owner session, valid IANA zone |
| /api/address-search | POST | Private owner session, bounded Photon search and signed canonical selections |
| /api/workload | GET, PUT | Private owner session, editable normalized rules |
| /api/workload/refresh | POST | Private owner session, aggregate learning and advice |
| /api/integrations/jaack-os/bookings | GET | Separate private read bearer, pagination and opaque identities |
| /api/integrations/jaack-os/bookings/:sourceId | GET, POST | Separate read/write credentials, revision/hash attribution and durable change journal |
| /api/integrations/jaack-os/booking-changes/:requestId | GET | Private read bearer plus exact source/hash attribution |

State keeps settings, legacy tokens, google_accounts with versions, hashed sessions, bookings with source IDs/revisions/pending changes, booking_slot_outcomes and jaack_os_booking_changes. Legacy columns are added only if absent. No applied schema is dropped or renamed. Settings JSON canonicalizes whole numbers for Node-compatible learning signatures.

When valid booking types are enabled, creation validates the requested/default type and writable destination, persists its account/calendar, and preserves that destination for later management even after rollback; inactive creation and legacy NULL calendars retain primary-calendar behavior.

Cryptographic compatibility: SHA-256 derives the AES-256-GCM key; encrypted storage is base64url of 12-byte nonce, 16-byte tag and ciphertext. Management links use the existing jev-booking-management-v1 HMAC domain and URL fragment. Signed Photon choices and JAACK OS request hashing remain compatible. Test secrets are synthetic.

Scheduling preserves host/viewer date boundaries, owner hours, workday clamping, DST, notice, busy conflicts/buffers, weekly focus/deadline policy, ranking and partial-month failure. Offered public slots are set by a 0–100 workload score (booked time against real free capacity 40, calls 5 each capped at 20, project switches 5 after the first capped at 15, travel or in-person 15, deadlines today or tomorrow 5 each capped at 10) with bands light 0–30 = 4, normal 31–55 = 2, busy 56–75 = 1, heavy 76+ = 0. Only in-person work events or trips (flights or travel) that overlap the workday close a day and add the travel score; events entirely before or after those hours do not. Locations containing Discord, Zoom, Teams, Meet, Telegram, online, virtual or a URL are virtual. Training-calendar and other personal or sport events (gym, tennis, runs) block only their own time plus the travel buffer and never close the day. The owner setting `loadMode` is one of normal (bands only), heavy (max 2 a day and 6 a Mon–Fri week) or ultraHeavy (max 1 a day and 3 a week); tier caps are configurable, an until date is optional, and the default is ultraHeavy. Offered slots are min(band, day cap, remaining week cap). The week cap is allocated to the lightest days first (lowest score, then earlier day), and the week count includes confirmed and pending bookings. `offCalendarLoad` minutes (weekly, per week, or per date; weekly values spread evenly over that week's weekdays; default 0) are added to booked time. TripIt flight/hotel inference applies date-specific host zones and buffered transit blocks to previews and fresh booking/change verification, with conservative fallback and the `JEV_TRAVEL_TIMEZONES=off|0|false` legacy-path rollback. Free/busy uses every accessible calendar, including hidden/shared calendars and freeBusyReader accounts. Event details from freeBusyReader calendars are never requested. Raw guest/event/project/address details are excluded from advisory aggregates and public workload responses.

Native requests retain Google Calendar, OAuth and Photon endpoints, pinned jev-1.13.0 aggregate workload questions, bounded independent confidence and deterministic fallback. Google Routes dispatch and travel advice are removed. Provider clients pool connections and bound timeouts. The public gateway retains a 30-second upstream timeout and never forwards owner cookies or bearer credentials.

Test-only fixture routes require DEMO_MODE and JEV_TEST_MODE. They remain outside the public allowlist. Real-provider injection accepts loopback only and is rejected with production permission. X-Jev-Backend: rust is the release readback marker.

Availability now intentionally differs from the captured Node oracle: travel routing is removed, Jev advice refreshes in the private background worker every six hours, and initial date selection uses the earliest eligible date. The month response adds optional firstDateAvailability and availabilityByDate fields with the same public fields as normal day responses. Browser previews expire sixty seconds after request start, are isolated by date/duration/viewer timezone, and are cleared on month reload. These preview results never bypass fresh booking conflict verification.
