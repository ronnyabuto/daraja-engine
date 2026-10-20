# Prompt 21: Sidecar gRPC and transport parity

Requirement IDs: R47
Prerequisite: stage 20 local acceptance; external capability gates remain separately enforced.

## Execution contract

Work in the assigned checkout of /home/officialnyabuto/Desktop/daraja-engine using GPT-6.1-Sol at medium reasoning. Model/effort selection is a Codex setting; prompt text cannot switch it. Implement only this numbered stage and stop when its acceptance criteria pass.

First read root AGENTS.md, docs/implementation/STATUS.md and docs/implementation/requirements.md; then the relevant PRD sections in docs/spec/prd-v1.0.txt (PDF is authoritative), protocol/state contracts and existing callers/tests. Verify the prerequisite stage statuses and required capability gates. Preserve the recorded branch/worktree and user changes. Use applicable installed skills; for new Git implementation work apply new-feature once, and current-docs only for uncertain version-sensitive behavior. Do not delegate or spawn subagents.

Make the actual implementation, not only a plan. Keep changes with the existing owner. Use synthetic local fixtures and real boundary tests; no live money movement or publishing without authorization. Never replace financial behavior with placeholder success, silently remove requirements/tests, invent provider guarantees or weaken acceptance to get green checks. Default unverified provider features off. Ask only about choices that affect financial correctness or required access; continue independent work while a real external gate is blocked.

Keep context bounded: inspect relevant ranges, retain contracts on disk, and finish one stage at a time. If the stage exceeds a safe session, checkpoint at a coherent substep, mark IN_PROGRESS and give the exact resume command/task; do not claim it is done. On a fresh session recover from disk, not assumed chat memory.

Before finishing, run proportionate affected checks, fix failures, remove redundant comments/scaffolding, and update STATUS.md, requirement implementation/test references and docs/implementation/evidence/21-daemon-grpc.md with actual commands/results and remaining blockers. Record code revision or dirty state. Mark DONE only for implemented and locally verified acceptance; distinguish BLOCKED live gates. Report changes, evidence, limitations and the next numbered prompt.

## This stage

Define and implement a versioned daemon-specific Protobuf service for the same core capabilities as REST. It is separate from the pinned upstream CDK service; do not overload or modify CdkPaymentProcessor to serve generic applications. Publish generated-schema provenance/versioning, typed amount/unit/destination representations, idempotency contract, errors, pagination/cursors and settlement streaming semantics.

Route gRPC and REST through shared existing application services with identical authorization and financial outcomes. Preserve Pending during unknown/manual states, exact receipt reporting and quote expiration behavior. Make stream disconnect/replay/backpressure/cancellation safe using durable events and bounded buffers. Configure gRPC listener, authentication and deployment TLS boundaries explicitly.

Acceptance: generated tonic client tests exercise collections, resend, payout quote/execute/status, balance and incoming stream; replay after reconnect and daemon restart. A compact parity suite submits representative equivalent REST/gRPC operations and compares stored state/amounts/results, including key reuse with changed payload and ambiguous payment. Provide a reproducible proto build/compatibility check. Document how clients connect without direct database access or in-process Rust dependencies.

