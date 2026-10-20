# Prompt 17: CDK settings, create/check incoming and stream

Requirement IDs: R03 R42
Prerequisite: stage 16 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/17-cdk-incoming.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Implement the pinned upstream CdkPaymentProcessor service in grpc.rs starting with GetSettings, CreatePayment, CheckIncomingPayment and WaitIncomingPayment. Map supported mpesa methods and KES/sat units exactly to generated upstream messages and current capability gates. Use the established core quote/settlement APIs; the processor must not duplicate financial state.

Preserve dual-rail checkout metadata, exact expected amount, expiry and retry_after through fields/options actually supported by the pinned contract. If metadata does not fit, document and implement an explicit compatible extension/reference-client lookup rather than changing upstream proto messages. Define stable quote identifiers and errors under retries.

WaitIncomingPayment must be backed by durable settled events with correct reconnect/replay/cancellation behavior within the upstream RPC's actual semantics. Never emit held/unverified deposits. GetSettings cannot advertise disabled units, unsupported workflows or unimplemented capabilities.

Acceptance: real tonic client/server tests for all four RPCs using generated pinned types and a real repository. Cover reconnect, duplicate CreatePayment, no-phone fallback, expired FX, held/forged deposits, backend errors and stream restart. Startup should report any still-unavailable outgoing capability honestly until stage 18; no fabricated success or NotImplemented paths in enabled incoming methods.

