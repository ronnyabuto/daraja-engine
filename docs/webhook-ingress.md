# Callback ingress

Owner: `daraja-core::webhook`; storage: `repository::Repository` and migration
`0003_callback_inbox.sql`. `webhook::router(repository, policy)` returns an axum
router for embedding in a service. The binaries do not yet start a service;
service lifecycle and workers belong to later stages.

| Endpoint | Admission/binding |
|---|---|
| `/callbacks/stk/{secret}` | `Body.stkCallback`; stored CheckoutRequestID and MerchantRequestID bind the quote and its configured collection shortcode. Success requires unique metadata names, exact positive KES amount, receipt, transaction date and phone field. Failure may omit metadata. No callback BusinessShortCode is required or invented. |
| `/callbacks/c2b/validation/{secret}` | Synchronous local quote/amount check, enabled only after operator verification described below. |
| `/callbacks/c2b/confirmation/{secret}` | C2B TransID/TransAmount/BusinessShortCode/BillRefNumber/TransTime/TransactionType/MSISDN shape. Collection shortcode must match. Account reference binds an existing quote; unmatched references remain untrusted orphan observations. Masked/hashed MSISDN is accepted and never used as identity. |
| `/callbacks/b2c/{secret}`, `/callbacks/b2b/{secret}` | Existing PayoutResult DTO; rejects contradictory receipt/amount parameters. |
| `/callbacks/b2c/timeout/{secret}`, `/callbacks/b2b/timeout/{secret}` | Existing StatusTimeout DTO; preserves ambiguous payout status. |
| `/callbacks/status/result/{secret}`, `/callbacks/status/timeout/{secret}` | Existing StatusResult/StatusTimeout DTOs. |
| `/callbacks/balance/result/{secret}`, `/callbacks/balance/timeout/{secret}` | Existing balance result/timeout DTOs (same provider envelopes as status). |

All routes accept POST. DTO parsing rejects duplicate JSON object keys, invalid
UTF-8, malformed JSON and invalid required fields. Amounts use checked decimal
KES parsing with no floating point rounding. Accepted callbacks remain **untrusted**.
No route calls confirm, settle, payout resolution, or receipt allocation. Neither
a URL secret nor a query/callback success authenticates payment evidence.
Status/balance/payout observations retain provider correlations, including early
results that arrive before acknowledgment persistence. Later workers must bind
those IDs to durable queries/operations; a supplied correlation is no authority.
Actual account/version payload support remains subject to operator verification.

## Secrets, bounds and network policy

`Policy::new` requires a distinct secret for every Endpoint::ALL entry. Generate
each secret independently with a CSPRNG, for example `openssl rand -hex 32`, and
supply it as `crypto::Secret`. Only 64 lowercase hex characters are accepted;
format checks cannot establish randomness. SHA-256 digests are retained and
compared with subtle's constant-time equality over fixed 32-byte digests. Provision
and rotate secrets outside source control; update registered URLs during rotation.
Never log populated registration objects, request URIs or raw request bodies.

Default body limit is 32 KiB; the total body/lookup/persistence deadline is 500 ms.
`with_limits` accepts positive sizes up to 1 MiB and positive deadlines up to two
seconds. Invalid secret returns generic 404, malformed DTO 400, body-limit failure
413, network policy rejection 403 and disabled validation/storage/deadline failure
503. Errors never echo request fields. The ingress adds no URI/body access logger;
SQL statement/binding logging stays disabled. Captured server TRACE tests cover
these paths, including malformed input. Configure TLS terminators, load balancers,
APM and any enclosing middleware to omit callback paths, query strings and bodies
from access/error logs; enclosing middleware can otherwise expose secrets before
this router runs. Log a static endpoint label and status only. Protect the database
and backups: the inbox intentionally retains sensitive provider payloads for
reconciliation; it is not a log or public API.

CIDR restrictions are optional and supplementary. `restrict_direct_peers` checks
only the TCP peer from axum ConnectInfo; serve with
`into_make_service_with_connect_info::<SocketAddr>()`. Missing peer information
fails closed when this restriction is enabled. An empty list denies all. There
is no built-in provider allowlist. Community CIDRs in the PRD are unofficial and
incomplete and are never accepted as an authoritative provider identity.

Forwarded, X-Forwarded-For and X-Real-IP headers are always rejected. Proxy-chain
source attribution is unsupported. For a reverse proxy, explicitly restrict the
network so only that proxy reaches ingress, strip forwarding headers there, and
understand that a direct-peer restriction then checks the proxy address. Provider
source filtering must happen at that trusted edge if desired; an internet caller
must not be able to bypass it or reach ingress directly. TLS and the URL secret
remain required regardless of source network.

## Durable acknowledgment and replay

Every accepted confirmation/result/timeout (including STK failure) commits a
canonical JSON observation and pending job in one short SQLite transaction before
returning `{"ResultCode":"0","ResultDesc":"Accepted"}`. This acknowledgment
means durable receipt only. The inbox ID hashes a versioned domain, static endpoint
and canonical JSON. Whitespace/object-key-order replays share an ID; changed claims
are separate observations even when they reuse a receipt. Exact duplicate delivery
returns success without duplicating work or reopening a completed job. Callback
secrets are excluded from stored identity and payload. An orphan's nullable quote
hint is a snapshot: workers must re-evaluate binding from the payload, not trust it.

`callback_inbox` and `callback_jobs` are separate from verified receipt allocation
and existing domain jobs, so spoofed observations cannot reserve a legitimate
receipt. Both survive reopen. A failed job insertion rolls back the observation;
a lost/late acknowledgment may lead to a safe duplicate delivery. Pending jobs are
durable work records; consumers, leases, evidence acquisition and restart worker
wiring remain stages 09/13/14. No no-op verifier or settlement authority is supplied.
STK/C2B amount mismatch, expired quotes and C2B orphans are retained for that later
exception workflow, without changing quote state or verified receipt allocations.

## C2B synchronous validation

Validation defaults **off**, returning 503. The C2B-VALIDATE capability register and
configuration template remain false. Registration alone does not activate provider
validation. Before calling `enable_verified_c2b_validation(allow_held_topups)`, the
operator must verify shortcode activation, the current response-code contract and
timeout/default-response behavior. Register with `C2bResponsePolicy::Cancelled` as
the candidate failure default and prove its behavior on the actual account; a
provider configured Completed, disabled validation or a timeout can allow payment
regardless of this endpoint. No local HTTP response guarantees provider rejection.
See the [primary-contract findings](protocol-contract.md#callback-fields-and-evidence-limits)
and [capability gates](verification-gates.md).

The enabled endpoint uses a consistent local database snapshot, without provider
I/O. It accepts an unexpired ISSUED/STK_SENT quote whose account and collection
shortcode match and whose FX snapshot, if any, is still valid. Exact expected KES
returns string ResultCode `0`; unknown/expired/nonactive/wrong-shortcode accounts
return `C2B00012`; invalid or different amounts return `C2B00013`. ResultDesc is a
static safe description. Malformed envelopes return generic 400. Validation does
not persist a payment observation, reserve funds or authorize settlement.

With the explicit held-top-up policy enabled, UNDERPAID_HELD may accept only
expected KES minus the sum of **verified allocated** receipts, while quote and FX
remain unexpired. Unverified callbacks never affect this sum. Other held/terminal
states reject, and the original full amount is rejected for a partly funded quote.
This is the admission rule for stage 10's exception workflow; refunds/re-quotes and
operator authorization remain there and in stage 15. Concurrent validations are
not reservations: two actual payments can still overfund a quote and must be held
by the independent accounting verifier rather than minting twice.

Reproduce the real TCP/file-backed admission, validation, rollback/reopen and
server TRACE checks with `cargo test -p daraja-core --test webhook --locked`.
