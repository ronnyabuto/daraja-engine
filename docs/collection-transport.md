# Collection and query transport contracts (stage 05)

`daraja-core::collection` owns STK processrequest, stkpushquery, explicit C2B URL
registration and asynchronous TransactionStatus. `transport` owns the injectable
HTTP boundary. Neither module changes financial state, allocates receipts, sends
payouts or supplies trusted settlement evidence.

## Routing and wire behavior

`CollectionClient<T, C>` requires a typed `CollectionApp`, `HttpTransport`, provider
root URL, `Clock` and STK shortcode/passkey. All three collection operations use
that actor. `TransactionStatusClient<T, C>` requires an explicit
`StatusCredentials::{Collection, Disbursement}` matching the account being queried;
there is no credential fallback. Operators still establish app/account permissions.

The exact paths and field spellings follow [protocol findings](protocol-contract.md):

| Method | Path | Contract |
|---|---|---|
| `submit_stk` | `/mpesa/stkpush/v1/processrequest` | EAT password/timestamp; `CallBackURL`; positive exact whole-KES `Amount`; matching PartyA/PhoneNumber and configured PartyB |
| `query_stk` | `/mpesa/stkpushquery/v1/query` | EAT password/timestamp and `CheckoutRequestID`; response checkout must match request |
| `register_c2b` | `/mpesa/c2b/v1/registerurl` | `ShortCode`, `ConfirmationURL`, `ValidationURL`, `ResponseType`; explicit operator invocation only |
| `TransactionStatusClient::request` | `/mpesa/transactionstatus/v1/query` | `Initiator`, encrypted `SecurityCredential`, `CommandID=TransactionStatusQuery`, `TransactionID` **or** `OriginalConversationID`, `PartyA`, `IdentifierType`, `ResultURL`, **`QueueTimeOutURL`**, `Remarks`, **`Occasion`** |

`C2bResponsePolicy` serializes `Cancelled` (default) or `Completed`. Registration
never runs during construction/startup and does not activate external validation.
Provider activation and actual failure/timeout behavior remain C2B-VALIDATE gates.
Callback URL validation requires HTTPS and rejects userinfo/fragments. Full URLs
are `Secret` values. Provider root URLs require HTTPS, except numeric loopback HTTP
fixtures; userinfo, query, fragment and non-root paths are rejected.

Receipt lookup is enabled by default. Original-conversation lookup fails closed
until explicitly enabled by the operator after recording STATUS-ID evidence for
that endpoint/environment. No gate register or example capability was enabled.
Provider idempotency/duplicate-rejection guarantees are unrelated and unchanged.

## Deadlines, observations and retry ownership

`ReqwestTransport::new(connect, request)` requires positive deadlines and
`connect <= request`. The request deadline covers response-body consumption as well
as sending and headers; responses are limited to 64 KiB. Errors distinguish connect,
request/header and body timeout/transport phases. DNS/TCP/TLS belong to connect;
the distinction uses reqwest's actual `is_connect()` classification. HTTP status
and `x-request-id` survive body failures. Redirects, underlying HTTP retries,
environment proxies and verbose wire logging are disabled. Alternate transports
must enforce the same safety contract; callers can inject a wrapper for testing.

Every operation uses Bearer authorization with a sensitive header and JSON content
type. One bounded OAuth refresh follows a 401, only to prepare future calls; the
original HTTP outcome is returned and that operation is never replayed by auth.
OAuth acquisition/refresh has its own stage 04 deadline. Thus an entire public
call can take longer than the individual provider HTTP deadline. Hooks and query
retry sleeps have separately owned lifetimes; workers must bound their own tasks.

`Outcome` retains original provider codes and available request, merchant,
checkout, originator and conversation IDs alongside HTTP status. IDs use redacted
Debug; errors omit provider descriptions, body text, nested transport errors,
URLs and credentials. A body `requestId` takes precedence over the optional header
request ID. Malformed responses retain available HTTP/header evidence. Successful
HTTP alone is insufficient: acknowledgment `ResponseCode` must be string/integer
zero and required endpoint correlations must be present. A nonzero STK query
`ResultCode` is a returned status observation, distinct from a rejected query.

Only `query_stk_with_retry` retries, and only for the exact provider spike-arrest
code `500.001.1001` (including within HTTP 200). `QueryRetryPolicy` permits at most
five retries, positive initial delay and a cap of at most 60 seconds. Full jitter
is sampled over nanoseconds from zero through `min(initial * 2^retry, cap)` using
OS entropy in `TokioQueryRuntime`. Injectable entropy/sleep makes the schedule
reproducible. Retry indices begin at zero. A zero-retry policy is allowed. Every
retry builds a fresh timestamp/password. Exhaustion returns the last original
outcome. Other errors, financial submission, registration and asynchronous status
requests never use this retry policy. The transport implements no 35/65/110-second
worker schedule or resend cooldown; those remain stages 08/09.

## Asynchronous persistence hooks

There is deliberately no default/no-op production implementation of
`StatusPersistence`. A consumer must implement both hooks:

1. `prepare(StatusPrepared)` must commit a unique local query ID, immutable lookup,
   creation time and its preconfigured quote/operation/callback-secret binding
   before returning. It runs before OAuth and status HTTP I/O. Reject reuse or
   conflicting prepared identity; never infer that a prepared row was not sent.
2. `acknowledge(local_id, ack)` must durably and atomically bind both provider IDs
   without replacing conflicting IDs or losing an early callback observation.
   It runs after successful acknowledgment parsing. A hook failure returns a
   persistence error retaining the acknowledgment's HTTP/correlation evidence.

Cancellation, HTTP error, missing/malformed acknowledgment, timeout and hook
failure preserve the prepared record as unresolved. They do not authorize a resend
or any terminal financial decision. Prepared callback bindings let ingress retain
an early callback until acknowledgment correlation arrives. Persistence hook calls
must perform short commits and never retain a transaction across provider I/O.
The hook test implementation records calls in memory solely to verify ordering
and failure behavior. Durable worker/schema wiring and restart reconciliation of
these status requests remain stages 09/13, not a stage 05 durability claim.

`StatusAcknowledgment` has correlation and transport outcome, **no final payment
result**. `StatusResult::parse` and `StatusTimeout::parse` read the separate `Result`
envelope. Timeout stays an unresolved observation. Result parameters retain
redacted values, reject duplicate keys, and optionally parse `Amount` and
`ReceiptNo`; conflicting receipt/transaction IDs fail closed. No amount or receipt
is invented when omitted. Arbitrary parameter values and descriptions are never
logged. JSON bodies reject duplicate object keys at every depth and floating-point
numbers. String/integer code and ID forms are accepted without dropping their
original string spelling. `parse_kes` accepts positive integer JSON numbers or
exact decimal strings (up to two fraction digits), uses checked minor-unit
arithmetic and rejects signs, whitespace, exponents, floats, excessive precision,
zero and overflow. A provider sending decimal numeric tokens instead of strings
requires a separately verified exact-decimal decoder; floats are not rounded.

These parsers do not authenticate callbacks or verify financial finality. Ingress
routing/secret checking belongs to stage 07 and independently trusted evidence to
stages 09/13/27. SETTLE remains disabled; no observation alone settles a quote or
releases a possibly dispatched payout.

## Verification and dependency evidence

Run `cargo test -p daraja-core --test collection --test redaction --locked`
to reproduce the [transport](../crates/daraja-core/tests/collection.rs) and
[TRACE redaction](../crates/daraja-core/tests/redaction.rs) checks.
All tests use synthetic loopback HTTP/TLS fixtures, including a stalled TLS
handshake to exercise the real connect deadline and a streaming body stall to
exercise the body deadline. Existing TRACE capture now includes outgoing STK,
C2B and status bodies with distinct secret markers.

Pinned reqwest is still 0.12.24; no dependency version/lockfile changed. Exact
[ClientBuilder](https://docs.rs/reqwest/0.12.24/reqwest/struct.ClientBuilder.html)
and [Error](https://docs.rs/reqwest/0.12.24/reqwest/struct.Error.html) documentation
fetches were unavailable. The installed pinned source verified `connect_timeout`,
whole-request `timeout`, `retry::never`, `redirect::Policy::none`, `no_proxy`,
`connection_verbose(false)`, `Response::chunk` and `Error::{is_timeout,is_connect}`.
The existing tokio-util dependency enables `io` solely for the streaming fixture;
no new crate/version was introduced.
