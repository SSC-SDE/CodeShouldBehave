# Plug-and-play AI extension: detailed implementation plan

Updated: 2026-11-14. Status: **planned; implementation and acceptance pending**.

This is the current detailed AI-extension plan for the maintained application.
Read with [implementation progress](implementation-progress.md) and the existing
[App execution boundary](APP_EXECUTION_DEVELOPMENT.md). It supplements, rather than
renumbers or completes, Phase 7 steps 7.1–7.8. The separately recorded nested HLD
implementation remains independent. No AI dispatch, model download, provider
credential, disclosure, deployment, subscription or launch is authorized here.

## 1. Product decision and first delivery

AI is an optional explanation layer over deterministic analysis, not an analysis
prerequisite. First delivery: **Explain this HLD boundary**, on explicit reviewer
action. Explain what changed, possible architectural implications, evidence-backed
review questions and unknowns for one selected, exact PR comparison.

Preserve the existing Center Stage / Focus template, Legend, Settings, themes,
comparison/zoom controls, source inspector, HLD hierarchy and LLD behavior.
Place the explanation in an additive disclosure beside the selected HLD boundary;
no replacement page shell, automatic full-PR generation or LLD redesign.

Initial provider: local Ollama, evaluating `qwen2.5-coder:7b` as a starting baseline,
not a quality guarantee. Record the resolved model digest and hardware before
qualification. A downloadable public model is not a free public inference API.
Remote providers, whole-PR summaries and optional LLD explanations require later
explicit scope and their own evidence; they are not advertised as implemented.

## 2. Non-negotiable extension contract

- Core HLD/LLD, code navigation, exports and existing checks work without AI,
  Ollama, provider credentials, a model download or an AI subscription.
- AI is disabled by default. Disabled mode performs no model call, health probe,
  model loading or background generation, including on startup or PR navigation.
- Installing/configuring a provider is separate from permission to disclose data.
  Every dispatch needs current repository/path, principal and destination grants.
- Input is an authorized immutable evidence bundle; output is advisory text and
  citations. AI cannot mutate graphs, scope, accepted design, checks or merge state.
- Provider failure, missing configuration, quota or AI storage failure affects
  only the AI disclosure. Never block core admission or mark analysis failed.
- Provider/model changes and disabling AI require no deterministic reanalysis,
  review-data migration, workspace redesign or loss of personal review state.
- Providers are trusted compiled adapters, not arbitrary executable plugins or
  browser-supplied URLs. Do not add an agent framework, tool execution or vector
  database for this feature.

## 3. Existing foundations and required additions

`src/ai.rs` already defines `ModelAdapter`, `DisabledModelAdapter`, a fixture-only
adapter, `EvidenceBundle`, destination/disclosure authorization and result checks.
`GET /v1/ai/capability` currently reports disabled; no real adapter is available.
The current result validator checks bundle/rule identity and citation indexes,
not structured claims or whether cited code actually supports an interpretation.

Reuse these boundaries. Add a feature-gated adapter under `src/ai/ollama.rs`, an
AI-only orchestration/evidence module, and bounded explanation rendering in the
existing workspace. Reuse the maintained HTTP client dependency where compatible;
do not add a provider SDK or make inference dependencies part of the default CLI.
An optional `ai-ollama` Cargo feature may depend on `server` and `dep:ureq`;
confirm dependency and runtime compatibility before freezing that feature.

Do not perform inference while holding the existing storage mutex/transaction or
on a parser worker. The current synchronous adapter contract needs an isolated,
bounded blocking worker or an explicitly reviewed async boundary, never unbounded
thread spawning. Keep AI admission and failures separate from core scheduling.

## 4. Proposed API and response contracts

These are proposed APIs, not implemented endpoints or compatibility promises.

- Extend existing `GET /v1/ai/capability`; do not create a competing capability
  route. Report `disabled`, `unconfigured`, `unavailable` or `ready`, provider/model
  metadata and supported features without exposing secrets. Disabled/unconfigured
  responses must be computed without contacting a model service.
- Add authenticated `POST /v1/reviews/{id}/ai/explanations`. Accept the exact HLD
  design/boundary identity and expected evidence identity. The server resolves
  evidence; reject arbitrary prompts, source payloads, provider URLs and scope
  expansion from the client. Initial requests are non-streaming and bounded.
- Return a validated explanation or a typed AI-only failure: access/missing,
  disabled/unconfigured/unavailable, busy/budget exhausted, timeout, stale evidence
  or invalid output. Preserve existing denial semantics; do not leak inaccessible
  review existence or upstream response bodies. Navigation can cancel the client
  request; cancellation never means model execution is proven stopped.
- Version the result schema: exact review/design/boundary and evidence identities,
  provider/model digest, destination-policy revision, prompt/schema/rule versions,
  generation settings/time, headline, change explanations, review questions and
  unknowns. Each factual change claim references bundle evidence IDs and a side.
- The server assigns presentation labels: `Observed` only for deterministic facts;
  model interpretation is `Inferred`; unsupported conclusions remain `Unknown`.
  A model cannot promote its own text to observed or attach a confidence score.
- Reject invalid JSON/schema, unknown properties, oversized lists/text, foreign or
  nonexistent citations, stale identities and disallowed markup. Citation validity
  and human-assessed claim support are separate gates. Render text as text, not HTML.
- Keep the existing Mermaid graphs deterministic. AI must not introduce new
  component/edge IDs, invent call order from imports or supply executable Mermaid.

## 5. Evidence, privacy and provider configuration

Build a minimal bundle from the selected baseline/proposal, HLD boundary delta,
relevant changed declarations and exact supporting source spans. Retain repository,
commit, blob, path, line range, producer/resolution and coverage per evidence item.
Do not send the full repository, unrelated files, credentials or hidden context.
PR descriptions/docs may supply separately labeled author intent, never authority.
Repository text and model responses are inert untrusted data, not instructions to
execute code, use tools, browse URLs or change permissions.

Before dispatch, preview actual destination, model, disclosure mode and exact
included paths/commits. Loopback inference still requires an approved endpoint and
compute policy. Symbols and summaries are sensitive too; redaction complements
scope authorization and is not a promise to remove every unknown secret.

Server configuration selects enabled state, provider, base URL, model and limits.
Initial Ollama destination: `http://127.0.0.1:11434`, called by the backend, never
the browser. A hosted server's loopback is not the reviewer's laptop. Reject
request-controlled destinations, redirects, ambient proxies and metadata-service
access. Remote/private destinations require a separate allowlisted policy; never
silently fall back from local to cloud. Keep the unauthenticated local Ollama
listener off the public network. Future keys belong in an approved secret store,
never prompts, logs, browser storage, source control or published bundles.

Call Ollama `POST /api/chat` with `stream: false`, a JSON Schema `format`, pinned
model identity and explicit generation settings. Enforce schema again server-side.
Detect context overflow; do not accept silent evidence truncation. No auto-pull,
auto-install or model switch when a configured model is unavailable.

## 6. Admission, identity, caching and lifecycle

Initial conservative limits below require operator approval and measurement before
real-data dispatch. They are ceilings, not measured performance claims, and do not
relax existing Phase 7 envelopes or publication budgets.

| Control | Initial local-only proposal |
| --- | --- |
| Concurrent inference | 1 globally, 1 per tenant; busy requests fail admission |
| Whole request deadline | 60 seconds including connection and response reading |
| Automatic retries / fallback | 0 / none |
| Evidence | At most 16 spans and 16 KiB UTF-8 content |
| Token budget | At most 4,096 input and 512 generated tokens |
| Response body | 64 KiB maximum before parsing |
| Request admission | 10 new generations per principal per hour |
| Explanation cache | Memory only; 128 entries, 16 MiB, 1-hour maximum TTL |

Qualify the exact model/tokenizer and context budget before dispatch; a byte cap
is not a token counter. If safe context admission cannot be established, keep the
adapter unavailable. Measure local CPU/RAM/latency and deterministic-review
performance; reject the deployment if inference starves normal review workloads.

Canonical AI identity must explicitly bind security domain/tenant, exact baseline
and proposal, comparison mode, scope/grant and context pins, boundary/design,
evidence content/blob/ranges, disclosure/destination policy, model digest,
prompt/schema/rules and generation settings. Audit existing bundle hashing and
version it where necessary; do not assume its current fields cover this identity.
Deduplicate authorized identical in-flight work without sharing results across
tenants. Recheck access before dispatch, returning a cache hit and rendering or
exporting a result. Model/prompt changes invalidate only AI entries.

Do not cache raw prompts/source or credentials. Explanation text itself may reveal
code, so apply access checks, byte limits, retention, logout/revocation purge and
an explicit clear action. Persistent AI caches are outside this first slice.
Revoke/disable prevents new dispatch and publication immediately; abort transport
best-effort. Unknown provider completion retains its admission reservation until
confirmed completion/cancellation or explicit bounded operator recovery. Never
blindly retry an ambiguous timeout or reset a request budget to recover it.

## 7. Ordered implementation checklist

These are work packages within `phase-7`, not new phases or new phase branches.
Inspect the existing branch/PR, ownership and predecessor gate/merge evidence
before coding; preserve the active nested-HLD work and unrelated backend edits.
Record actual tested SHAs, commands, outcomes and reviewer acceptance per package.
Planning approval does not start implementation, pass a gate or authorize merging.

1. **Contract freeze (7.1):** approve local-only scope, owners, disclosure policy,
   exact limits, model/hardware and versioned request/result schemas. Map the
   broader 7.1–7.8 obligations; hosted adapters/commercial beta remain pending.
2. **Isolation (7.2):** add disabled-by-default configuration, capability states,
   explicit consent/payload preview, endpoint restrictions and separate admission.
   Prove zero model egress and unchanged core behavior before live model calls.
3. **Evidence and identity (7.2–7.3):** build bounded selected-boundary bundles,
   canonical identities, stale/revoked-request rejection and adversarial fixtures.
   Run exported-symbol reference checks before changing the existing AI contract.
4. **Local adapter (7.3):** implement the optional Ollama transport, structured
   output, context/deadline/body caps and honest cancellation/error handling.
   Qualify it against explicitly approved local fixtures, not private PRs by default.
5. **Application integration (7.4):** add the explanation endpoint, independent
   cache/single-flight logic, reauthorization and sanitized diagnostics. Demonstrate
   that turning AI off or stopping Ollama cannot fail deterministic review calls.
6. **Focus integration (7.4):** add Explain this boundary, payload preview and
   separately contained loading/error/stale states within the existing template.
   Rapid boundary/PR changes must never display a response for the previous selection.
7. **Evaluation and opt-in pilot (7.6):** assess a versioned representative corpus
   before enabling real data; document unsupported cases and a rollback drill.
   Local qualification does not pass the broader Phase 7 beta/commercial gates.
8. **Delivery decision:** record exact supported capabilities and disabled modes;
   obtain separate rollout/disclosure approval. Keep default AI-off and preserve
   the existing deterministic installation and report workflow.

## 8. Required acceptance evidence

- **Core independence:** default build/install and HLD → LLD → exact source work
  without the feature, configuration or model service; no AI startup/health egress.
- **Failures:** absent model, stopped Ollama, malformed/truncated output, timeout,
  context overflow, saturation and cache failure affect only the AI disclosure.
- **Isolation:** two-tenant access, changed grants, cache hits after revocation,
  fabricated citations, stale commit/boundary, secret sentinels, prompt injection,
  redirect/proxy/metadata targets and unknown completions fail closed.
- **UI:** original Focus/Standard structure and preferences remain; node navigation,
  Legend, Settings, themes, zoom, source and unmodified LLD work on desktop/mobile,
  with keyboard/focus, cancellation and out-of-order-response browser evidence.
- **Quality:** start with at least 12 authorized representative comparisons covering
  boundary changes, contract changes, unchanged HLD/private LLD change, incomplete
  coverage, removals and misleading author intent. Separate prompt-development
  cases from held-out human judgments; record supported-claim rate, critical
  misleading claims, citation validity, reviewer usefulness, latency and usage.
- **Pilot gate:** zero unauthorized disclosures/actions or accepted fabricated
  citations/critical unsupported claims in that corpus. Before tuning, assign an
  evaluator and freeze numeric usefulness/support/latency thresholds; unresolved
  thresholds block real-data rollout. This finite pilot does not replace the
  broader 100-case Phase 7 evaluation or prove universal semantic correctness.
- **Rollback:** disable AI and purge its memory cache; core artifacts, review state,
  routing and deterministic checks remain usable without migration or reanalysis.

## 9. Hosted/static delivery and deliberately deferred work

The published Pages workspace is static: no direct browser-to-Ollama call, embedded
key or unauthenticated inference proxy. On-demand remote inference needs the
separately authenticated application service. A later public-report option may
pre-generate approved AI annotations in a separate trusted job and publish a
versioned, validated immutable artifact; preserve the deterministic analysis ID,
existing signed manifests, size/readback gates and all unselected reports.
Do not add model networking to the networkless reviewed-code analysis sandbox.
Public disclosure needs separate approval; withdrawal cannot recall copied content.

Ollama Cloud, hosted/BYOK adapters, streaming, persistent AI history, subscriptions,
whole-PR narratives, LLD explanations and broader beta remain unimplemented until
separately qualified. A provider swap uses the same extension contract but must
pass its own auth, privacy, schema, cancellation, usage and quality tests. Current
Ollama Cloud documentation does not promise local JSON-Schema output enforcement;
never infer capability parity or silently substitute a cloud model.

Official references, checked during planning; recheck at implementation:
[chat API](https://docs.ollama.com/api/chat),
[authentication](https://docs.ollama.com/api/authentication),
[cloud capabilities](https://docs.ollama.com/cloud),
[model tags](https://ollama.com/library/qwen2.5-coder).
