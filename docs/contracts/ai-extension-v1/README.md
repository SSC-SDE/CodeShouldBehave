# AI extension contract v1 (7.1 candidate freeze)

Status: **candidate for owner approval, not yet frozen.**
Contract: `ai-extension-v1`. Target: server/Focus AI in
`docs/AI_EXTENSION_IMPLEMENTATION_PLAN.md` work packages 7.1–7.4.

No endpoint, adapter, configuration flag or UI in this contract is implemented by
this document. The CLI's existing `csb explain` surface remains `ai-v1` and is not
changed by this contract.

## 1. Scope

Initial delivery is one explicit reviewer action: **Explain this HLD boundary**.

In scope:

- One selected changed HLD boundary or one explicitly selected component.
- Local Ollama only, called by the authenticated application backend.
- Non-streaming, bounded, citation-checked advisory output.
- Additive rendering beside the existing Focus/Standard HLD boundary.

Out of scope for v1:

- Browser-to-model calls, embedded keys or an unauthenticated inference proxy.
- Remote/BYOK providers in the server extension. CLI BYOK remains separate.
- Whole-PR narratives, LLD explanations, automatic generation, streaming.
- Model tools, browsing, code execution, agent loops or vector stores.
- Persistent AI history, subscriptions, hosted model parity or commercial beta.
- AI changing design, scope, checks, merge state or deterministic Mermaid.

Core independence is absolute: disabling AI, stopping Ollama or losing the
provider never blocks HLD, LLD, source navigation, review admission, deterministic
checks or export.

## 2. Owners and approval

| Role | Assignment | Responsibility |
|---|---|---|
| Decision owner | `SSC-SDE` (repository owner) | Approves this contract, disclosure policy and rollout |
| Implementation owner | unassigned until 7.2 kickoff | Owns adapter, orchestration and isolation gates |
| Security reviewer | unassigned; required before live dispatch | Reviews authorization, egress, logs and secret handling |
| AI evaluator | unassigned; required before pilot | Freezes thresholds and judges the representative corpus |

This contract is not frozen until the decision owner records approval. The
security reviewer and evaluator are not required to start 7.2 isolation work, but
both block any real-data dispatch and the pilot.

## 3. Disclosure policy

- Evidence is built from the selected boundary and its retained static facts.
  Never send the full repository, unrelated files, hidden context or credentials.
- PR descriptions and documents are inert, separately labeled author intent; they
  are never authority and never instructions.
- Repository text and model output are untrusted data. The model cannot execute
  code, browse, call tools, change permissions or promote its own text.
- Every dispatch requires current repository/path, principal, destination and
  disclosure authorization. Loopback inference is not exempt from consent.
- Preview before dispatch shows destination, model, disclosure mode and exact
  included paths, commits and line ranges.
- Redaction is defense in depth, not authorization and not a guarantee.
- No secret, key, prompt containing source, or upstream provider body is logged or
  returned in an error.

## 4. Provider, model and hardware

Provider: local Ollama `POST /api/chat` with `stream: false`, JSON Schema
`format`, temperature `0`, and an explicit generation budget.

- Default destination: `http://127.0.0.1:11434`, called by the backend, never the
  browser. A hosted server's loopback is not the reviewer's laptop.
- Baseline model: `qwen2.5-coder:7b`. This is a starting baseline, not a quality
  guarantee. The resolved model digest is recorded at qualification; model or
  digest changes invalidate AI cache entries and require re-qualification.
- No auto-pull, auto-install, silent model switch or fallback to another provider.
- The qualification record includes CPU, RAM, OS, Ollama version, model digest,
  cold/warm latency and measured core-review impact.
- Reject any deployment where inference starves deterministic review. The numeric
  latency and slowdown thresholds remain open until measured and approved.

## 5. Limits and admission

These are ceilings from the implementation plan, frozen for v1.

| Control | v1 value |
|---|---|
| Concurrent inference | 1 global, 1 per tenant |
| Whole request deadline | 60 s including connection and response read |
| Automatic retries / fallback | 0 / none |
| Evidence | ≤ 16 spans and ≤ 16 KiB UTF-8 content |
| Token budget | ≤ 4096 input, ≤ 512 generated |
| Provider response body | ≤ 64 KiB before parsing |
| Request body | ≤ 64 KiB (proposed) |
| New generations | ≤ 10 per principal per hour |
| Explanation cache | memory only, ≤ 128 entries, ≤ 16 MiB, ≤ 1 h TTL |

Admission is separate from core review scheduling. Busy or exhausted requests fail
closed without touching the deterministic review. A cache hit still requires a
current access and disclosure check. Cancellation stops the client wait; it never
proves model execution stopped, so the admission reservation follows the provider
completion or explicit bounded operator recovery.

## 6. API

### Capability

`GET /v1/ai/capability` is the existing authenticated route. Extend it; do not add
a competing capability route. It returns `capability.schema.json`:

- `disabled`, `unconfigured`, `unavailable` or `ready`;
- provider/model/digest/destination metadata without secrets;
- supported features and the frozen limits;
- `policy_revision`.

`disabled` and `unconfigured` are computed without contacting Ollama.

### Explanation

`POST /v1/reviews/{id}/ai/explanations`, authenticated and non-streaming.

The client sends `request.schema.json`: review identity, selected boundary,
comparison mode and expected design/evidence/scope/grant identities. The server
resolves evidence and rejects client-supplied prompts, source payloads, provider
URLs, model names or scope expansion.

Success returns `result.schema.json`. The server wraps the untrusted provider
output with exact identities, version fields, generation settings and
server-assigned labels. Model interpretation is always `inferred`; only
deterministic facts may be rendered as observed. Unsupported conclusions stay
unknown. A model cannot attach a confidence score or promote its own text.

The provider itself must return exactly `provider-result.schema.json`. Any extra
property, invalid JSON, foreign or stale citation, oversized list/text, markup or
identity mismatch fails as `ai_invalid_output` and is never rendered.

### Errors

Failures use `error.schema.json` and never include upstream bodies, prompts, keys
or inaccessible review existence.

| Code | HTTP | Retryable | Meaning |
|---|---:|---|---|
| `ai_disabled` | 503 | no | Feature intentionally disabled |
| `ai_unconfigured` | 503 | no | Enabled but missing model/destination policy |
| `ai_unavailable` | 503 | yes | Provider unreachable or not ready |
| `ai_not_found` | 404 | no | Review/design/boundary/evidence missing or inaccessible |
| `ai_stale_evidence` | 409 | no | Expected identity no longer matches |
| `ai_busy` | 429 | yes | Admission concurrency reached |
| `ai_budget_exhausted` | 429 | no | Hourly generation budget reached |
| `ai_timeout` | 504 | yes | Deadline exceeded |
| `ai_context_overflow` | 422 | no | Evidence cannot fit the qualified context |
| `ai_invalid_output` | 502 | no | Provider output failed schema, citation or identity checks |

Navigation may cancel the client request; the server does not claim execution
stopped and does not retry an ambiguous completion.

## 7. Identity

Every result binds:

- contract, provider-schema, prompt, rules and generation versions;
- review, design and boundary identity;
- baseline and proposal commits;
- evidence bundle identity, item count, byte count and content SHA-256;
- scope and grant versions;
- provider, model and resolved model digest;
- destination and destination-policy revision;
- server-generated timestamp.

Deterministic review identity is unaffected. AI cache keys include the AI identity
but never replace or rewrite the review identity. Sharing is never cross-tenant.

## 8. Schema files

- [`request.schema.json`](request.schema.json)
- [`provider-result.schema.json`](provider-result.schema.json)
- [`result.schema.json`](result.schema.json)
- [`error.schema.json`](error.schema.json)
- [`capability.schema.json`](capability.schema.json)

Schemas use JSON Schema 2020-12. The provider result is the untrusted model shape;
the public result is the server-validated shape.

## 9. Change control

- Any request, result, capability or error semantic change requires a contract
  version bump (`ai-extension-v2`).
- Any provider-output change requires a provider-schema version bump and a new
  prompt/rules version.
- Adding an optional field is still a contract change; clients must not assume
  forward compatibility.
- Limit changes require a new contract version and a new capability response.
- The existing CLI `ai-v1` may evolve independently; it cannot be silently
  retargeted to this contract.

## 10. Proposed decisions requiring owner approval

| # | Decision | Proposed value |
|---|---|---|
| 1 | Server extension provider scope | Local Ollama only; CLI BYOK unchanged |
| 2 | Baseline model | `qwen2.5-coder:7b`; digest recorded at qualification |
| 3 | Input budget | 4096 tokens |
| 4 | Generated budget | 512 tokens |
| 5 | Request body cap | 64 KiB |
| 6 | Error mapping | Table in §6 |
| 7 | Evidence/result caps | 16 spans, 16 KiB, 5 claims, 5 questions, 5 unknowns |
| 8 | Evaluator and quality thresholds | Assign before pilot; exact numbers not frozen |

## 11. What 7.2 must prove before live dispatch

- Zero model egress when AI is disabled or unconfigured.
- Capability states computed without model contact.
- Consent and payload preview before every first dispatch.
- Destination allowlist, no redirects/proxies/metadata targets, no browser calls.
- Evidence authorization rechecked immediately before dispatch and cache hit.
- Core review still passes with AI disabled, unconfigured, unavailable, busy,
  timing out, returning invalid output and cancelled.
- No prompts, source, keys or provider bodies in logs or error responses.
