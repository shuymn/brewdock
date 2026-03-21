<!-- Audience: coding agents. Direct instructions, not tutorials. -->

# ADR 0001: Nanobrew-style install method resolution

## Context

Current brewdock baselines assume exact-tag stable bottle installs only and reject formulae with `post_install` or missing host-tag bottles. That blocks representative `homebrew/core` formulae such as bottle-tag mismatches, formulae requiring lightweight `post_install`, and formulae that need a small source fallback.

The target environment remains Apple Silicon macOS under `/opt/homebrew`. The project still does not want Homebrew Formula DSL compatibility, arbitrary Ruby execution, tap support, cask support, or Linux/Intel runtime expansion.

## Decision

Adopt a nanobrew-style install pipeline with three ordered methods:

1. Prefer a compatible stable bottle selected by fallback tag order.
2. If `post_install` exists, fetch the `homebrew/core` Ruby source, parse it with `ruby-prism`, lower only allowlisted AST shapes into internal operations, normalize reachable filesystem effects into fixed schemas, and execute only those lowered operations after relocation and before linking.
3. If no compatible bottle exists, resolve a generic source build plan and run a minimal build driver.

The compatibility bottle order for Apple Silicon is fixed to:

- `arm64_sequoia`
- `arm64_sonoma`
- `arm64_ventura`
- `all`

All runtime paths continue to flow through `Layout`; no short prefix such as `/opt/nanobrew` is introduced.

Source/post-install metadata is read from formula JSON plus `homebrew/core` Ruby source fetched from `raw.githubusercontent.com/Homebrew/homebrew-core/HEAD/<ruby_source_path>`.

Unsupported `post_install` nodes that remain in the macOS runtime branch, unsupported requirements, or unsupported source build systems fail closed. On any post-install or source-build failure, keg cleanup is required and receipt/state DB writes do not occur.

The initial `post_install` lowering surface is fixed to receiverless space-call / paren-call, zero-arg helper method calls, receiver-based path helper calls, `if OS.mac?`, `if path.exist?`, `rm(..., force: true)`, `rm(...) if ...exist?`, `install_symlink`, and `Formula["..."].pkgetc`.

The evaluator does not execute AST directly. The initial internal operation set is fixed to:

- `Mkpath`
- `CopyFile`
- `RunSystemArgv`
- `RemoveIfExists`
- `InstallSymlink`
- `IfPathExists`
- `MacOsBranch`
- `CallHelper`
- `ResolveFormulaPkgetc`

macOS runtime semantics are fixed as follows:

- `if OS.mac?` executes only the `then` branch
- `else` branches are parsed but are not runtime execution paths
- non-runtime branches may be used only as static normalization material

Helper methods are restricted to zero-arg calls. Path helpers must lower to a single path expression, action helpers must lower completely into allowlisted operations, and recursive helpers, non-zero-arg helpers, or block-taking helpers fail closed.

Schema normalization is fixed to two generic filesystem-effect schemas:

- bundle bootstrap: materialize a keg PEM bundle into `prefix/etc/<formula>/cert.pem`
- dependent cert symlink: replace `prefix/etc/<name>/cert.pem` with a relative symlink to `Formula["..."].pkgetc/"cert.pem"`

Formula-name matching and string-fragment builtins are not part of the supported design.

## Rejected Alternatives

- Ruby shim / zerobrew-style execution
  - Rejected because it reintroduces Ruby runtime coupling and broadens the supported surface beyond the intended minimal subset.
- Formula-specific `post_install` builtins
  - Rejected because they leave user-visible support dependent on formula-name matching instead of the reachable filesystem effect, making representational variance brittle.
- Full Homebrew Formula DSL compatibility
  - Rejected because it is out of scope, raises implementation and verification cost sharply, and is unnecessary for the targeted failing formula set.
- Exact-host-tag bottle-only policy
  - Rejected because it leaves common Apple Silicon formulae unsupported despite safe compatible bottles or viable source fallback.

## Consequence

`brewdock-formula` becomes responsible for bottle selection inputs and install method planning inputs, not only exact-tag supportability.

`brewdock-core` owns a single install method resolution path reused by install, dry-run, and upgrade, plus source fallback orchestration.
Bottle execution is staged behind an explicit execution plan so `network acquire`, bounded pre-finalize local prepare, and Homebrew-visible finalize remain separate contracts instead of ad-hoc control flow.

`brewdock-cellar` gains Prism-backed parse, restricted lowering, and schema normalization support for post-install execution and becomes part of the fail-closed boundary around receipt/state persistence.

The system accepts a larger subset of `homebrew/core` formulae while keeping unsupported behavior explicit and bounded.

## Revisit trigger

- Need to support tap formulae or casks
- Need to execute Ruby beyond restricted AST lowering and schema normalization
- Generic source driver cannot satisfy acceptance formulae without formula-specific DSL behavior
- `/opt/homebrew` ceases to be the required installation prefix
