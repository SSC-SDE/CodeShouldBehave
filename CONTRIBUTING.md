# Contributing

CodeShouldBehave is a deterministic Rust review engine with a browser workspace.
The public CI profile analyzes explicit public Git objects; it never builds or
runs the pull request under review. AI is not required.

Start from an issue or a focused change with a reproducible expected result.
Keep unsupported, incomplete and deferred outcomes explicit. A structural graph
does not prove runtime behavior, and a reviewed-file tick is not merge approval.
Do not broaden context, permissions, telemetry or provider access incidentally.

Use Rust 1.87 or later, Node.js with its built-in test runner, and Python 3.11+
for release packaging. The analyzer itself does not need Python or Node.

```sh
cargo +1.87.0 test --locked --all-features
cargo +1.87.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.87.0 clippy --locked --all-targets --no-default-features -- -D warnings
cargo +1.87.0 fmt --check
node --test tests/*.test.cjs
python3 -m unittest discover -s tests -p 'public_*test.py' -v
```

For public export or UI work, include source/scope/commit tampering and a complete
HLD → LLD → exact-source case, not only a snapshot of the new markup. For CI work,
include fork-controlled workflows, quota refusal, old heads finishing last,
withdrawal/recovery and honest check conclusions. Run the affected checks first;
record the exact source and failures. Never generate a load storm against GitHub.

In the original development repository, follow `AGENTS.md` and the phase tracker:
exact phase branches, one integration PR per phase, no direct `main` pushes and
no inferred merge permission. Do not overwrite another contributor's work.

Keep changes reviewable: explain the problem, resulting behavior, validation and
remaining limits. Update usage and compatibility documentation for user-visible
changes. Retain third-party licenses and attribution; add dependency/version
changes to the inventory rather than treating every component as MIT.

Do not put credentials, retained private IR, account dumps, screenshots containing
private data or private repository excerpts in an issue or PR. Use the security
policy for a sensitive finding. Contributions are submitted under the project's
existing MIT license unless an alternative is explicitly agreed before inclusion.
