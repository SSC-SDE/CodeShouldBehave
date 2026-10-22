# Third-party notices

CodeShouldBehave's own source is licensed under [MIT](LICENSE). Third-party
components retain their own licenses and attribution. This file does not
relicense them.

The approved workspace distributes the unmodified Mermaid tiny 12.1.0 bundle
from the official `@mermaid-js/tiny` package. Its SHA-256 is
`143c1b9345c45963ff429da4bdcd9051316a43522ea9edd7ccb7a87ff947a804`.
Its embedded license comments are preserved.

| Component/notice embedded in that bundle | License text and attribution |
| --- | --- |
| Mermaid | [MIT text](docs/prototypes/vendor/mermaid-LICENSE.txt), copyright Knut Sveidqvist |
| DOMPurify 3.4.12 | [Upstream license text](licenses/javascript/DOMPurify-3.4.12-LICENSE.txt), Cure53 and other contributors; bundle declares Apache-2.0 and MPL-2.0 |
| Lodash ES 4.18.1 | [Upstream license text](licenses/javascript/lodash-4.18.1-LICENSE.txt), OpenJS Foundation and contributors; retains Underscore attribution to Jeremy Ashkenas, DocumentCloud and Investigative Reporters & Editors |

Version-specific license origins and hashes are recorded in
[sources.json](licenses/javascript/sources.json). The package build also copies
the renderer's MIT text alongside its own assets. Keep this notice, those texts
and the renderer's embedded notices together when distributing the workspace.

The native parser also retains the [Tree-sitter 0.25.10 MIT notice](licenses/native/Tree-sitter-0.25.10-LICENSE.txt)
from its verified upstream release commit, and the [SQLite 3.51.3 notice](licenses/native/SQLite-3.51.3-NOTICE.txt)
from the checksum-verified bundled source. These supplement the registry archive's
nested notices; Unicode's notice alone does not replace Tree-sitter's own MIT
notice. [Native origins](licenses/native/sources.json) bind them to locked package
versions/checksums, and packaging refuses a stale supplemental notice.

Rust binary distribution must include the license inventory and original license,
notice and copyright files generated from the exact Cargo.lock packages. Lockfile
inventory includes platform/test dependencies and is not a claim that each is
linked into every executable. SQLite and other bundled native code retain their
upstream notices. A missing package/license is a packaging failure, never silently
treated as MIT. Source dependencies remain pinned by Cargo.lock checksums.

Publication audit and release acceptance remain separate from writing this file.

The CodeShouldBehave workspace bundles unmodified Inter and JetBrains Mono variable fonts
under the SIL Open Font License 1.1. Their original copyright and license texts
are in [src/fonts/inter-OFL.txt](src/fonts/inter-OFL.txt) and
[src/fonts/jetbrains-mono-OFL.txt](src/fonts/jetbrains-mono-OFL.txt). Font source
URLs and content hashes are recorded in [src/fonts/README.md](src/fonts/README.md).
They are served locally; the browser does not contact a font CDN.
