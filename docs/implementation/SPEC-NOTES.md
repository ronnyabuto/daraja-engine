# Questions the implementation must resolve

These are planning observations from the supplied PRD, not established upstream/provider findings.

| Question | Required treatment | Owner stage |
|---|---|---|
| One workspace vs two product names | Follow section 5.1: core, processor and daemon crates in one workspace. | 00 |
| CDK seven-RPC schema and custom mpesa/KES/sat support | Pin exact upstream revision, generate real schema and run a real mint integration. Do not invent fields. | 00–01, 17–19 |
| B2C/B2B duplicate OriginatorConversationID behavior | Verify each independently with environment-specific evidence; unverified auto-resubmission remains off. Duplicate does not mean paid. | 01, 13, 27 |
| TransactionStatus without a receipt | Verify accepted identifiers. Missing queryable ID means Pending/manual recovery, never a guessed query or new transfer. | 01, 13 |
| Unsigned asynchronous query callbacks | Correlation/URL secret is not a signature. Establish explicit evidence/trust model and unresolved deployment restrictions. | 01, 09, 13, 27 |
| STK callback missing shortcode or query missing receipt/amount | Bind stored attempt/config where supported, document precisely what the provider attests; do not manufacture fields. | 01, 07, 09 |
| Receipt insertion before independent verification | Keep resumable unverified observations distinct from confirmed allocations so spoof/crash cannot poison the unique ledger. | 02–03, 09 |
| Exact C2B validation vs partial-payment top-up | Define allowed remaining amount for an approved held quote; avoid permanently rejecting recovery payments. | 07, 10 |
| Validated C2B correlation alternative vs mandatory query | Default to the stronger mandatory independent verification policy; any exception needs documented evidence and an explicit decision. | 01, 09 |
| MAX_PENDING_TTL called a hard cutoff | Cut off automated work/escalate; retain Pending proof lock until evidence-backed resolution. | 02, 13, 15 |
| Two rails or old/new STK attempts both receive funds | One quote settlement; extra verified value is an accounted exception/credit, never a second mint event. | 02, 08–10 |
| Required quote fields unsupported by pinned proto | Use supported metadata or documented companion lookup; preserve upstream wire compatibility. | 17–19 |
| Frozen fees/rates and concurrent float commitments | Version quotes and use exact arithmetic, TTLs and transactional reservations. | 11–14 |
| ~15MB static binary / 60-second integration | Measure binaries and exercise examples. Treat these as targets, not guarantees. | 24, 26 |
| Legal section claims October 2026 rules | Verify with official sources/counsel; distinguish draft, enacted and unknown. Do not promote PRD assertions into legal clearance. | 01, 27 |
| Optional sweeps, PostgreSQL, policy choices | Implement PostgreSQL; sweep stays opt-in with linked accounts and verified support. Choose held overpayment default and configurable KES grace. | 10, 14, 16 |

