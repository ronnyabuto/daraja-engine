# Verification gates

Reviewed 2026-10-20. Machine register: [verification-gates.json](verification-gates.json).
Every gate is disabled. The register and `config.example.toml` are checked by
`python3 scripts/test-protocol-gates.py`; the service binaries still refuse startup.
Neither file is a runtime authorization system. Later configuration/startup code
must enforce these gates before exposing financial capabilities.

## Evidence classes

`LOCAL_ONLY` means synthetic tests: useful for our request bytes, error handling
and invariants, never provider guarantees. `SOURCE_TRACED` describes inspected
pinned code, not an executed outcome. `UPSTREAM_LOCAL` means executed pinned CDK
tests with a real mint/proof store and synthetic payment backend.
`DOCUMENTED_NOT_LIVE_VERIFIED` is a primary provider description, not observed
account behavior. `SANDBOX_OBSERVATION` applies only to the tested sandbox app,
endpoint and time. Production evidence must be recorded separately.
`UNVERIFIED`, `UNRESOLVED_TRUST`, `BLOCKED_ACCESS` and `BLOCKED_OPERATOR_COUNSEL`
never authorize a capability. There is no automatic path from mock output or a
probe response to an enabled flag.

## Required evidence to open a gate

| Gate | Owner / current evidence | Required release evidence |
|---|---|---|
| LIVE | Operator; no credentials or execution authorization supplied | Correct environment credentials, product/shortcode roles, approved scope and destinations; all dependent technical and legal gates satisfied. |
| DUP-B2C | Stages 13/27; current B2C docs describe duplicate code; no live trial | Independent B2C v3 evidence: scope, retention window, duplicate concurrent/serial behavior, exact same bytes + same caller ID, conflict semantics, authoritative provider ledger proving no second transfer. |
| DUP-B2B | Stages 13/27; ID acceptance and guarantee unverified | Independent B2B v1 evidence for **both** BuyGoods and PayBill commands, including whether caller ID is accepted at all. B2C results cannot pass this gate. |
| STATUS-ID | Stages 05/13/27; documented `OriginalConversationID` request | Test receipt-only and original-conversation-only queries separately; determine caller/proxy/provider ID accepted, required PartyA/role, not-found vs indeterminate outcomes, mapping to the original payment. Never query with a guessed spelling or ID. |
| SETTLE | Stages 07/09/13/27; unresolved authenticity and field attestation | Provider-approved ingress provenance with explicit threat model; trusted receipt, exact amount, beneficiary/shortcode, finality and quote bindings. A correlated unsigned result or successful STK query with missing receipt/amount cannot pass. |
| C2B-VALIDATE | Stages 07/10/27; enablement/codes documented | Provider activation for exact shortcode, registered URLs, rejection/acceptance and unreachable/timeout cases; approved `Cancelled` fallback, bounded response time and remaining-amount top-up policy. Verification still applies to received deposits. |
| CDK-LOCK | Stages 18/19; wire fixture, source trace and upstream-local regressions | Actual pinned client + processor + mint: make/check gRPC errors, severed connection after possible dispatch, lost stream, process kill/reopen persisted DB, delayed Paid, definitive Failed. Assert proof states and no second payout. |
| CDK-KES | Stages 17/19; custom-unit source/types | Actual mpesa/`kes` mint routes/settings/keyset configuration, issue/spend/melt with exact amounts. No accidental sat conversion. |
| CDK-SAT | Stages 17/19; built-in unit and custom-method source/types | Actual mpesa/`sat` issuance/melt, FX TTL and rounding/reservations; unsupported wallets remain unsupported. |
| SWEEP | Stages 14/27; no account evidence | Linked-account entitlement, command/endpoint approval, float reconciliation and restricted authorization. Do not infer support from an unrelated top-up command. |
| LEGAL | Operator/counsel; dated sources below, no deployment clearance | Written classification for the actual entity, product, token, custody, jurisdictions, accounts, customers and flows; applicable licences/exemptions and ongoing controls. |

Before automatic resubmission, **both** endpoint families need their independent
gates passed. Production behavior is not established by sandbox behavior. A safe
trial first persists the immutable operation, exact body and caller ID before
I/O; compares whole body bytes and ID on a repeat; records proxy/provider IDs
separately; and reconciles the provider ledger afterward. A duplicate error only
records duplicate rejection. It authorizes neither Paid nor Failed. A mismatch
aborts the repeat. An unknown outcome stays Pending even after manual escalation.

## Sandbox probe operation

`scripts/sandbox-probe.py` is an experiment tool, not a reconciler. No background
worker, automatic retry, receipt settlement or gate editing exists. Offline
planning is the default and requires no credentials:

```sh
python3 scripts/sandbox-probe.py b2c --payload /tmp/operator-approved-b2c.json
```

Use a locally protected payload file (excluded from Git) and an explicit
`DARAJA_SANDBOX_ACCESS_TOKEN` environment variable obtained through the provider's
sandbox OAuth flow. Never put credentials on the command line. The operator must
verify that the token, shortcode and all recipients belong to the approved
sandbox experiment. No credential discovery or production URL option exists.

After **separate explicit user authorization** for the specific experiment:

```sh
python3 scripts/sandbox-probe.py b2c --payload /tmp/operator-approved-b2c.json --execute --authorize-sandbox-money-movement
# A separate authorized duplicate experiment, never an automatic retry:
python3 scripts/sandbox-probe.py b2c --payload /tmp/operator-approved-b2c.json --repeat-of /tmp/first-b2c-payload.json --execute --authorize-sandbox-money-movement
```

The same tool has separate `b2b-buygoods` / `b2b-paybill` modes and `status`,
`stk-query`, `c2b-register` experiments. Each invocation makes at most one POST
to a hardcoded HTTPS sandbox path. TLS verification stays enabled; redirects and
environment proxy inheritance are disabled; timeout is 20 seconds, response
limit 64 KiB. Callback URL values must use HTTPS even though the sandbox docs
permit HTTP. Whole-KES positive payout amounts and command/identifier spelling
are checked; the harness is not a complete provider schema validator.

Output contains endpoint label, environment, payload hash and allowlisted numeric
response/error codes, never bodies, response descriptions, full callback URLs,
MSISDNs, receipts or credentials. Transport exceptions are suppressed into an
indeterminate diagnostic. No payment can settle from this output. A blocked or
ambiguous request is never retried automatically. Protect original payloads and
provider records separately; output hashes support equality checks but cannot
establish provider semantics or prove dispatch. An accepted HTTP response alone
cannot pass a live gate. Preserve authorized experiment scope, revisions, exact
times, payload equality result, correlation mapping and independently obtained
ledger outcomes in restricted operator evidence; commit only reviewed redactions.

No money-moving probe was executed in stage 01. Missing credentials block these
external trials, not stage 02's independent local implementation.

## Operator and counsel verification tasks (PRD section 2)

This register documents evidence and tasks, not a legal opinion. Deployment
classification must be specific to the operator; the PRD's "software shield"
does not establish an exemption for anyone distributing or operating this system.

| Topic | Current official evidence / classification | Required task |
|---|---|---|
| Existing payment law | CBK publishes [National Payment System Act No. 39 of 2011](https://www.centralbank.go.ke/wp-content/uploads/2016/08/NATIONAL-PAYMENT-SYSTEM-ACT-No-39-of-2011-21.pdf), including PSP authorization in section 12. Its [payment oversight page](https://www.centralbank.go.ke/national-payments-system/) describes the Act and 2014 Regulations as the oversight framework. | Counsel determines e-money/PSP treatment of KES bearer claims, third-party merchant redemption, safeguarding, authorization and any claimed closed-loop exclusion. Do not infer one from software licensing. |
| NPS 2026 policy / Bill | CBK's [21 September notice](https://www.centralbank.go.ke/2026/09/21/draft-national-payment-system-policy-and-national-payment-system-bill-2026/) labels these drafts. The [official public notice](https://www.centralbank.go.ke/uploads/press_releases/873447122_Public%20Notice%20-%20Draft%20National%20Payment%20System%20Policy%20and%20National%20Payment%20System%20Bill%2C%202026.pdf) invites submissions by 9 October 2026. | Monitor enactment/commencement; consultation closing does not enact the Bill. Do not treat proposed sandbox admission or exemptions as current authority. |
| VASP Act | [Kenya Law official source](https://new.kenyalaw.org/akn/ke/act/2025/20/eng%402025-11-04/source) identifies Act 20/2025, assent 15 October, publication 21 October, commencement 4 November 2025. Indexed official text was available; direct retrieval returned 403. | Obtain current consolidated law; classify SAT mint/FX bridge, custody, payment processing, anonymity-enhancing service restrictions, competent regulator(s), licence scope and section 48 transition applicability. A 2026 date is not universal grace for new operators. |
| VASP 2026 Regulations | CMA hosts the [final Gazette instrument](https://www.cma.or.ke/download/34/regulations/6377/the-virtual-asset-service-providers-regulations-2026.pdf): Gazette Supplement 185, LN 134, dated 22 July 2026. This is distinct from Treasury's March draft. Download SHA-256: `8ff0f1f80ea9811122daefabdd832eee0e0ef5a193c20d7cb02e13293e112ee6`. | Counsel verifies current amendments, commencement, category and application. Earlier Kenya Law direct fetches failed; the final primary instrument was subsequently obtained through CMA. |
| PRD capital table correction | Final Fifth Schedule, Gazette page 4367 (PDF page 115), visually checked: payment processor paid-up capital KES 10m, **liquid capital 100% of current liabilities for at least 30 days**. PRD's 4m/8% processor figure is contradicted. Its listed paid-up amounts for broker/tokenization 10m, manager/ICO 20m, exchange 100m, wallet 150m and stablecoin 300m match those rows, but omit distinct liquidity conditions. | Apply the actual category-specific requirements after counsel classification. The table also distinguishes token issuance platform from tokenization. Do not hardcode the PRD processor liquidity formula. |
| Corporate onboarding | [Safaricom B2B page](https://www.safaricom.co.ke/lipa-na-m-pesa/business-to-business-b2b-payments) requires an existing shortcode, application and eligible organization. [Daraja FAQ](https://developer.safaricom.co.ke/faqs) describes business-admin/validation setup. | Operator obtains the current exact checklist: incorporation/CR12/KRA PIN, board authority, settlement account, head office/admin and API roles, product approval and allowed destinations. PRD's categorical exclusion of informal groups is not independently established for every product/entity. |
| Reversals and operational approval | Maker/checker availability is not proof of an irreversible settlement guarantee. | Obtain provider confirmation of actual reversal policy, maker/checker roles, customer reversal dispute exposure, tariff/limit terms and account linkage. Add legal/AML/KYC, sanctions, privacy/data retention and customer recourse controls required for the actual business. |

All operator/counsel tasks remain open. Record signed approvals and sensitive
account evidence outside public source control, with a reviewed reference in the
release evidence. Mocks and source inspection cannot pass these tasks.

## Local reproduction

```sh
cargo +stable test --workspace --locked --offline
python3 scripts/test-protocol-gates.py
python3 scripts/check-foundation.py
# Real upstream tests; obtain the exact source pin first, outside this repo:
git clone --depth 1 --branch v0.18.2 https://github.com/cashubtc/cdk.git /tmp/daraja-cdk-v0.18.2
python3 scripts/check-cdk-upstream.py --source /tmp/daraja-cdk-v0.18.2 --protoc /absolute/path/to/protoc
```

The runner verifies exact CDK HEAD and unchanged tracked source/lockfile, uses
upstream Cargo.lock and limits build jobs. `--offline` works only after required
dependencies are cached. It executes upstream unit/proof-state tests, not a
Daraja provider simulation or full cdk-mintd deployment. Actual results and
limitations belong in [stage 01 evidence](implementation/evidence/01-protocol-gates.md).
