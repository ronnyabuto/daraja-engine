# Durable inbound checkout

Owner: `daraja-core::checkout::CheckoutService`, using the existing inbound domain,
collection client and SQLite repository. This is a library service; daemon and CDK
adapters remain later stages. Callers must enforce their authentication and quote
access policy before invoking it.

`create(CreateCheckout)` accepts a caller-scoped `RequestKey` with operation
`inbound-create`, a positive whole-KES gross collection amount represented as
`Amount(KesMinor)`, and an optional normalized `254…` phone. Configuration supplies
the collection shortcode, positive TTL, explicit operator tariff snapshot,
settlement hold duration and secret callback URL. Expected M-Pesa amount and credit
are distinct: KES credit equals gross amount minus the explicit collection fee.
Stage 11 adds `create_with_oracle` for requested sat credit backed by whole-KES
collections, strict frozen FX expiry and explicit deployed mint capability
attestations. Missing oracle or capability rejects new sat requests. See
[tariff and FX contracts](tariffs-fx.md) for rounding, enabled-unit advertisement,
provider freshness limits and fee accounting.

The creation transaction commits the quote, request hash/binding, optional first
STK attempt and its poll job together. Account references contain eight uppercase
ASCII letters/digits, sampled without modulo bias from OS entropy. A unique index
and serialized writer transaction enforce collision safety across processes; a
collision generates a new candidate, bounded at 32 attempts. Exhaustion, entropy
and storage failures return errors. Request hashes include the operation,
shortcode, requested amount/unit and phone under the `inbound-v2` request contract;
generated identities, clock and pricing configuration are excluded. Legacy
configuration-bearing hashes are recognized from frozen quote/audit records.
Reusing a caller/operation/key with a different payload conflicts; matching concurrent requests return the original
quote and never dispatch again. A replay returns its current checkout projection.

The committed response contains quote ID, payment state, credit/unit, nullable
attempt ID/status/CheckoutRequestID, immediate fallback (PayBill, account reference,
exact minor-unit amount and whole KES), absolute UTC `retry_after` and quote expiry.
A manual-only quote has no attempt or cooldown. JSON uses the existing exact
`Amount` representation and enum names; timestamps are Unix seconds. No phone or
callback URL appears in the response or Debug output.

With a phone, a Tokio task invokes `CollectionClient::submit_stk` once after commit.
The initial response is captured before scheduling network work and exposes the
persisted DISPATCHING attempt immediately. `checkout(quote_id)` reads subsequent
acknowledgment/status updates, including MerchantRequestID and CheckoutRequestID
stored by the repository. A late acknowledgment still binds immutable provider
correlations after manual settlement, without regressing PAID or a terminal
attempt result. Network, HTTP, provider-rejection or malformed-response
ambiguity becomes UNKNOWN; only local input/configuration/capability rejection
marks an attempt FAILED without sending. `UnavailableCollection` supports manual
checkout when no STK transport is configured. All outcomes preserve fallback and
leave financial state unchanged except the existing STK_SENT acknowledgment.

Cancellation, process exit or failure to persist an acknowledgment can leave
DISPATCHING without a checkout ID. The persisted poll job remains unresolved;
neither restart nor idempotent replay sends it again. This stage does not implement
restart workers, task supervision or automatic repair. Stage 09 owns recovery of
unresolved work, including operator reconciliation where no provider ID survived.
Daemon shutdown supervision is stage 20. Provider acceptance of the collection
account and callback URL remains an operator prerequisite.

`resend(quote_id, key, phone)` uses operation `inbound-resend` and stays under the
same quote/reference/terms. The phone may differ from the first phone. Before I/O,
the server rejects `now < retry_after`, nonactive states and expired quotes. The
first poll deadline and resend eligibility both use the persisted attempt's start
plus 35 seconds, including after closing/reopening SQLite. Equality is eligible
for reconciliation; it is not proof of nonexecution. A resend may take a bounded
provider query while the already-created manual fallback remains usable.

Every previous attempt with provider correlations is reconciled over the
collection query client, checking both checkout and merchant IDs. COMPLETED or
unresolved results block a new prompt and do not settle. A failed callback alone
cannot authorize resend. Terminal failure-code semantics require explicit
`StkReconciliationPolicy::verified_failure_codes` operator attestation for the
endpoint/environment. Its default set is empty (STK-RESEND gate disabled).
Current primary pages did not expose a usable failure-code contract; fixture codes
are synthetic, not verified provider behavior. Attempts that failed locally before
I/O need no provider reconciliation. A possibly sent attempt with no checkout ID
blocks resend. New attempts are committed using the reconciled quote version;
concurrent attempts, receipt transitions or acknowledgments invalidate stale work.
Only the winning preparation dispatches. Local acknowledgment CAS retries never
repeat provider I/O.

STK result observations supply no settlement authority. Every attempt remains
associated with its quote; both older CheckoutRequestIDs and the manual reference
bind to it. C2B admission and verified receipt allocation never require the payer's
phone to match the initiating phone. Existing transactional allocation/settlement
rules hold excess receipts and permit one settlement event per quote. See
[state contracts](state-machines.md) and [callback ingress](webhook-ingress.md).

Reproduce the synthetic loopback HTTP/file-backed SQLite checks with:

```sh
cargo test -p daraja-core --test checkout --locked
```

The tests prove local durability, ordering, fallback and accounting boundaries.
They do not prove provider failure semantics, independent settlement evidence or
live payment acceptance. All [external gates](verification-gates.md) remain off.

Stage 09 consumes checkout's persisted STK jobs using the absolute 35/65/110-second
schedule. See [inbound reconciliation](inbound-reconcile.md) for provider evidence
limits and durable settlement notifications. Checkout resends retain their own
explicit reconciliation policy.
