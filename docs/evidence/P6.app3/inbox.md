# APP-3 authenticated inbox foundation

November 6, 2026. **Partial local implementation; activation off.**

Implemented: raw-body HMAC validation, duplicate-header/body/delivery checks,
numeric installation/repository/check-owner validation, durable minimal metadata,
60-second leases with attempt fencing, bounded capacity, and sticky access blocks
for removal/privacy/uninstall/suspension. Intake never creates an analysis grant,
fetches source, schedules PR analysis or publishes a check. No webhook was configured.

Tests in the all-feature Rust suite cover invalid signature, exact duplicate,
changed-content delivery collision, unknown installation/wrong App/check owner,
restart, lease expiry/reclaim, stale completion, removal/uninstall, delayed provider
access, non-restoration by unsuspend, inbox exhaustion preserving safety blocks,
and the actual HTTP route/header contract. Notification-only tests assert zero
analysis jobs and zero source reads.

Remaining APP-3 exit evidence: installation-token/current-PR reconciliation, fair
analysis queue integration, exact baseline/fork/base-retarget policy, full isolated
PR worker binding, resource measurement and restart/revocation through publication.
Claiming a notification is not proof that any of these are complete. APP-4 receipt,
publication and check delivery are also pending.
