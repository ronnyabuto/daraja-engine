# Evidence

Store concise per-stage records named NN-stage.md plus reproducible synthetic fixtures/scripts where warranted. Do not commit credentials, callback URL secrets, real customer MSISDNs, wallet seeds, bearer tokens or unredacted production payloads.

Record:

- Stage and exact code revision (or explicit uncommitted state).
- Requirement IDs and concrete acceptance outcomes.
- Tool/runtime/provider environment and dependency revisions.
- Commands actually run, exit results and observed persistent state.
- Evidence class: local unit, mock provider boundary, real CDK integration, live sandbox, deployment-specific provider evidence, operator/counsel.
- Failures fixed and checks rerun; skipped checks with reasons.
- Remaining gates, safe capability defaults and exact next action.

A copied expected output is not an observed result. HTTP acknowledgment is not settlement; mock duplicate rejection is not a provider guarantee. Keep large/generated logs out unless needed to reproduce a consequential failure.

