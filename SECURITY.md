# Security policy

The public CI profile accepts only explicitly opted-in public repositories and
approved paths. Published source excerpts are public downloads and cannot be
recalled. Private repositories/context require the separate local/private profile.

Report security-sensitive findings through **GitHub private vulnerability
reporting** on the repository's Security tab when that feature is enabled. Do
not post credentials, private source, exploit payloads or account details in a
public issue. Include the affected release/commit, configuration, expected and
observed behavior, and a minimal synthetic reproduction without real secrets.

Private reporting must be enabled and its actual destination verified before
the public beta launches. It is not yet claimed active by this policy file. If
the Security tab has no private reporting action, do not disclose sensitive
details publicly; the release's reporting channel remains unavailable until
the repository owner enables and verifies it. No email address or response SLA
is invented here.

Important boundaries:

- The trusted analyzer and workflows are pinned independently of a reviewed PR.
  Reviewed scripts, hooks, filters, submodules, migrations and services never run.
- The public bundle is an explicit projection, not a copy of private artifact
  JSON, SQLite, Git objects or retained LLD input.
- The publisher validates source scope, immutable identities, files, hashes,
  provenance and current eligibility before publishing source or a check link.
- Unavailable, partial, deferred, stale and failed results remain distinct from
  complete analysis. Static no-change does not mean semantic equivalence.
- Account hard-zero budgets are an operating precondition. Unknown/quota-failed
  publication does not purchase credits or switch to paid infrastructure.

If a release is compromised, disable the installed workflows, stop new
publication, pin a separately verified release and apply the withdrawal ledger
before recovery. Deleting a report cannot recall copies already downloaded.
Rotate any exposed credential at its issuer; deleting it from Git is insufficient.

Only releases with completed public-profile acceptance are supported for that
profile. Local preparation, a passing unit suite or a development build is not a
hosted/private tenant isolation or public deployment certification.
