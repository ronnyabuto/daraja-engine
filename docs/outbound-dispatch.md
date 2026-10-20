# Durable outbound dispatch

`daraja-core::outbound::OutboundPayments` orchestrates one initial B2C/B2B
submission per durable operation. It consumes a saved immutable `PaymentQuote`
through `MakePayment { key, quote_id }`. `key.operation` must be `payout`;
callers retain the same caller/key on every retry. The quote supplies normalized
destination, principal, tariff and exact fee reserve. There is no caller override
of those terms. This library is not yet a CDK RPC or daemon service.

## Admission and send authority

The operation UUID is deterministic: SHA-256 of `daraja.payout.operation.v1\0`
and the caller, operation and key, each prefixed by its u64 big-endian byte
length. The first 16 digest bytes become an RFC variant UUID with version 8.
The UUID is also the original caller `OriginatorConversationID`. The original
caller request and both proxy/provider IDs have separate stored fields, even
when the caller request equals the operation UUID.

A single SQLite writer transaction binds the request hash **and exact payload**,
revalidates the saved quote/expiry, and commits DISPATCHING, frozen quote/body,
command/endpoint, correlation, principal-plus-fee reservation, audit and recovery
job. Only a newly inserted operation authorizes `PayoutPersistence::prepare` to
return successfully. A replay returns its original payout; conflicting payloads
fail. Unique quoted-payout admission also rejects a second key for the same quote,
including through the lower-level quoted repository API. Rejected admission rolls
back the binding/reservation and makes no payout request.

The existing [payout client](payout-transport.md) performs one submission with no
retry loop. Neither repeated MakePayment nor a reconciliation job grants another
send. B2B command/caller-ID and 01-phone gates remain enforced by that client and
default off. Ordinary local 2547 fixtures do not establish production account
permissions. No sweep or refund execution is wired by this stage.

Replays read the saved operation before expiry validation or wire construction.
They preserve original JSON bytes, initiator, encrypted SecurityCredential,
callback URLs, amount and identity across restarts, certificate rotation or
configuration reload. OAuth bearer tokens are obtained separately and are never
stored in the business body. The endpoint is frozen with the dispatch; any future
approved same-ID probe must use its original endpoint/body and separately enforce
current endpoint/environment evidence gates. This stage exposes no probe API.

## Acknowledgment and callback races

One writer transaction binds the provider OriginatorConversationID/ConversationID,
stores the bounded transport outcome (HTTP/code/request/transaction IDs), correlates
staged inbox observations and audits the change. Conflicting IDs cannot replace an
existing binding. A late acknowledgment preserves an independently established
terminal state. On a database write failure, the transaction rolls back and the
service reports a repository error; the durable DISPATCHING record stays Pending.

[Callback ingress](webhook-ingress.md) durably commits observations before HTTP
acknowledgment. Payout admission now links candidates by rail and original caller
ID, or by both acknowledged proxy IDs. Once a provider conversation is known,
original-ID callbacks must match it too. Proxy callbacks arriving before the
synchronous acknowledgment stay in the inbox and are linked in its transaction.
A lost acknowledgment with unknown proxy IDs can still need external correlation;
the implementation does not invent that binding. Candidate links are untrusted;
early original-ID links do not authenticate callback claims. No callback settles,
releases a reservation or supplies definitive non-execution evidence.

Both early and late matched QueueTimeOutURL observations put a Dispatching/Pending
payout into UnknownInFlight; manual and terminal states do not regress. Duplicate
callbacks preserve one inbox/link record. Results with success or failure codes
remain observations for the later independent reconciliation boundary.

## Recovery and limits

Startup/supervision calls `Repository::recover_dispatching_payouts(now)` to mark
remaining DISPATCHING records UnknownInFlight without sending anything. It is
idempotent and preserves bodies, IDs, reservations and reconciliation jobs. A crash
before the actual send is deliberately indistinguishable from a crash after send.
A replay before explicit recovery still projects DISPATCHING as CDK Pending.

HTTP 500/503, socket disconnect, request/body deadlines, malformed acknowledgments
and provider 200 errors remain UnknownInFlight/Pending. A literal duplicate error
is Pending and never proof of payment. Transport `ProvenRejection` is recorded
separately only under the configured endpoint/command-specific evidence policy;
that policy defaults empty. This saga conservatively awaits the existing final
attestation boundary even for that observation. HTTP status alone cannot authorize
Failed. A storage error is also never a terminal financial status.

Migration 0007 adds quote uniqueness, immutable dispatch admission,
`payout_dispatch_observations`, `payout_callback_links` and inbox correlation indexing.
Legacy unquoted repository fixtures remain supported. Migration fails closed if
older data contains multiple quoted payouts for one quote; review such records
without deleting financial history. No live provider guarantee is inferred from
SQLite or mock tests.

Stage 13 owns status reconciliation and any separately evidence-approved probe;
stage 14 owns liquidity admission and stage 18 the CDK adapter. All external
[capability gates](verification-gates.md) remain disabled. See
`cargo test -p daraja-core --test collection outbound_dispatch --locked --offline`
for loopback provider/webhook and file-SQLite acceptance, including abrupt child
process exits on both sides of the send.
