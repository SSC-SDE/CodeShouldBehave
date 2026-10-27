# Live demo: scheduler retry policy extraction

This intentionally unmerged PR changes real scheduler code. It is a review
example, not a simulated dashboard or an engine deployment. PR #15 stays open.
The hosted controller analyzes the PR with the existing signed engine; it does
not execute this proposed code or require merging it.

## The story to tell

A burst of failed analysis jobs currently retries on identical 1/2/4/8-second
intervals. The scheduler also has no explicit provider Retry-After class.
This proposal separates pure retry decisions from lease-checked SQLite writes.

| Case | Baseline | Proposal |
| --- | --- | --- |
| Transient failure, attempt 1 | Retry after exactly 1,000 ms | 1,000–1,250 ms, stable per job |
| Provider asks for 30 seconds | No dedicated rate-limit class | Wait at least 30 seconds |
| Provider asks for over 60 seconds | No dedicated rate-limit class | Stop for operator review; never retry early |
| Fifth attempt / terminal failure | Failed | Failed |
| Cancellation | Cancelled | Cancelled |
| Timestamp overflow | Saturated retry timestamp | Failed, no immediate retry loop |
| Wrong worker or superseded lease | Refused | Refused before applying policy |

The new package is local, dependency-free Rust. It introduces no network calls,
credentials, service accounts or paid infrastructure. `RateLimited` is an API
capability for callers; this PR does not wire a new HTTP provider adapter.
The new enum variant requires downstream exhaustive matches to be updated.

## HLD rollout dependency

Local preflight found that the current signed engine truncates dependency nodes
when this repository exceeds the HLD storage budget. A separate engine fix must
retain dependencies before the HLD portion below can be accepted live. LLD has
both versions of `fail`; this PR stays unmerged while that engine fix is deployed.
The expected route below is not yet a claim of live acceptance.

## Three-minute presenter route

1. Open this PR's **CodeShouldBehave / Architecture** check and click
   **Open CodeShouldBehave report**. The exact report opens in **Focus**.
2. Choose **HLD**, then **Split**. Show the application gaining the
   `csb-retry-policy package` dependency. In Unified, inspect its added dependency
   and import relationships. These are code/package relationships, not a claim
   that a new running service exists. The approved application component remains
   the same; this PR does not change repository scope or confirmed boundaries.
3. Choose **LLD** and open `Scheduler::fail` in `src/scheduler.rs` (the label may
   be `fail`). Use **Split**: baseline computes backoff inline; proposal classifies
   rate limits, delegates to `retry_decision`, and persists the decision.
4. Select a changed branch/call to show baseline and proposal source at their
   exact commits. Both versions of `fail` exist, unlike the added-only PR #15.
5. Return to **Modules** and open `retry_decision` in
   `src/retry_policy/src/lib.rs`. Show cancellation, exhausted attempts,
   over-budget delay, deterministic jitter, and overflow branches. This new
   function correctly has no baseline counterpart.
6. Explain the visible coverage warning: static evidence and unresolved calls
   are not runtime verification, nor is the check merge approval.

## Reproduce code validation

```sh
cargo +1.87.0 test --locked -p csb-retry-policy
cargo +1.87.0 test --locked --all-features --lib
cargo +1.87.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.87.0 clippy --locked --all-targets --no-default-features -- -D warnings
cargo +1.87.0 clippy --locked -p csb-retry-policy --all-targets -- -D warnings
```

The policy unit tests verify retry bounds, stable staggering, Retry-After,
exhaustion and overflow. Scheduler transaction tests verify persisted deadlines,
claim timing, rejected worker ownership, and existing supersession fencing.
