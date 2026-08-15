# Par language and codebase overview

This document is a starting point for coding-agent sessions working on Par. It
summarizes the language model, standard library, implementation architecture,
validation baseline, and known limitations of this checkout.

It was prepared by reading, in order:

1. The complete English book under `docs/`.
2. All 29 standard-library modules under `crates/par-builtin/packages/`.
3. All 22 example programs under `examples/`.
4. The compiler, runtime, workspace, CLI, LSP, playground, package manager,
   test runner, and documentation generator.

The checkout inspected was version `0.1.0`, based on commit
`56e57727522e3fa3a843ec417a6c42b9efc5d93b` dated 2026-08-15, with the
WebSocket work on the `websocket` branch included.

## Executive summary

Par is a process language presented with expression syntax. It is based
directly on classical linear logic and CP, the process calculus from
"Propositions as Sessions." Except for primitives, values are best understood
as communication endpoints governed by protocols.

Its main design properties are:

- Linear ownership describes complete communication protocols, not only memory
  ownership.
- Every logical connective has a dual, so construction on one endpoint is
  destruction on the other.
- Independent computations are concurrent by default; dependencies impose
  sequencing.
- The tree-shaped process topology rules out cyclic channel waits within the
  language model.
- Recursive and iterative types separate finite recursion from productive
  corecursion.
- Errors are ordinary values rather than exceptions.
- Droppable linear resources receive compiler-generated structural cleanup.

The central design is implemented and well tested. The standard library builds
streams, parsers, HTTP, SQL, persistent maps, time handling, and shared linear
cells from these mechanisms. The surrounding product remains experimental:
dependency management is not reproducible, parts of the CLI and LSP are
unfinished, release automation appears stale, and the runtime/FFI boundary
still relies on Rust invariants and panics.

## Language model

### Values, channels, and processes

Par expressions are convenient syntax for building channels and processes.
Creating a channel with `chan` starts a process and exposes its dual endpoint.
Linking with `<>` connects compatible endpoints.

A channel's type is a protocol. Sending, receiving, selecting, or handling a
branch consumes the current endpoint and advances it to its continuation type.
This is why the same linear type system covers functions, data, objects,
resources, and session-typed concurrency.

The fundamental process operations are:

- `chan`: create a process and obtain its dual endpoint.
- `<>`: link two endpoints.
- `channel(value)` / `channel[value]`: send and receive.
- `.branch` / `.case`: select and handle protocol branches.
- `!` / `?`: finish and consume a unit-like protocol.
- `begin` / `loop`: construct iterative values.
- `.begin` / `.loop`: consume recursive values.
- `do`: expression-oriented process syntax.

`do { commands } in expression` is conceptually a channel whose process runs
the commands and links its result endpoint to the final expression.

### Duality

The principal source-level duals are:

| Constructive side | Dual/consumer side | Meaning |
| --- | --- | --- |
| `!` | `?` | terminate / wait for termination |
| `(A) B` | `[A] B` | send a value / receive an argument |
| `either` | `choice` | select a variant / offer handlers |
| `recursive` | `iterative` | consume finite structure / produce a potentially unbounded structure |
| `(type a) T` | `[type a] T` | provide a hidden type / accept any type |
| positive primitive | internal dual primitive | provide / request primitive data |

Pairs and functions are therefore dual views of the same connective. The same
is true of eithers and choices, recursive and iterative types, and existential
and universal types.

Functions are receiving channels. Choice types act like structural interfaces,
and iterative choices act like stateful objects or services.

### Linearity and constraints

Values are linear by default and must be consumed exactly once. Local variable
use is destructive: after a linear value is consumed, it is no longer in the
typing context.

Generic constraints form this chain from strongest/narrowest to
weakest/broadest:

```text
signed -> number -> data -> share -> drop
```

- `drop`: the value can be disposed of safely.
- `share`: the value can also be copied and reused.
- `data`: the value is comparable and displayable structural data.
- `number`: generic zero, addition, multiplication, and division are available.
- `signed`: subtraction and negation are also available.

The `drop`/`share` distinction is important. A resource may have deterministic
cleanup without being safe to duplicate.

`box T` is shareable regardless of whether `T` is shareable. Boxes are the
mechanism for reusable functions, persistent objects, and shared capabilities.

### Structural types and subtyping

Types are structural; aliases do not introduce nominal identities.

Subtyping includes:

- Primitive relationships such as `Nat` to `Int`, `Char` to `String`, and text
  to bytes-compatible data.
- Width subtyping for eithers and choices in opposite directions because they
  are dual.
- Variance derived structurally from each connective.
- Constraint-aware universal and existential binders.
- Safe box/share promotion.

Nominal abstraction must be achieved through module visibility or existentially
hidden representations rather than opaque named types.

### Generics

Par supports explicit first-class universal and existential types using
`[type a] T` and `(type a) T`.

It also supports implicit item-local type binders such as `[<a> a] a`. Each
implicit type is inferred from its immediately associated value argument. This
keeps inference local but is intentionally less general than whole-expression
or higher-rank inference.

Named type definitions may have parameters, but their parameters cannot carry
constraints. Constraints belong on operations that use a type.

### Recursion, corecursion, and totality

Par separates:

- `recursive` types: finite inductive structures.
- `iterative` types: productive coinductive protocols that may continue
  indefinitely.

`begin`/`loop` and `.begin`/`.loop` are not unrestricted recursion. The checker
tracks recursive loop identities and whether a loop proceeds through a
structurally smaller value or a valid descendant of an iterative consumer.

Global type aliases and definitions must also form acyclic dependency graphs;
general recursion through global names is not allowed.

`unfounded` locally disables the termination/productivity obligation. It is
used in `examples/src/QS.par`, `examples/src/NestedParsing.par`, and three test
implementations. Totality is therefore a strong checked default rather than an
unconditional guarantee.

### Automatic concurrency and deadlock freedom

Data dependencies enforce sequencing; otherwise independent parts of an
interaction net can proceed independently. There is no source-level
`async`/`await` distinction.

The deadlock-freedom argument is structural. Channel creation and linking keep
the process communication graph tree-shaped, ruling out cyclic waits between
well-typed Par processes. This guarantee does not cover buggy Rust externals or
violations of runtime invariants.

The current runtime is a single cooperative net reducer. External futures run
concurrently through `FuturesUnordered`, but interaction-net rewriting is not a
parallel CPU evaluator. Automatic concurrency should not be interpreted as
automatic multicore speedup in the current implementation.

### Cleanup

Choices can mark one branch with `*`, conventionally `.close*` or `.cancel*`.
Types with a valid structural disposal strategy satisfy `drop`.

When a droppable value is unused, type checking inserts an internal `Close`
command. At runtime cleanup:

- erases primitives and unit,
- closes both sides of pairs,
- closes selected either payloads,
- invokes the marked branch of choices,
- follows finite recursive structures,
- discards shared references cheaply.

It does not recursively traverse iterative structures, which could be
unbounded.

### Errors and conditions

Errors are ordinary values, normally `Try<error, result>`. `try`, `catch`,
`throw`, and `default` are local syntax sugar and do not introduce exception
unwinding.

This is important because concurrently evaluated expressions do not
necessarily form a meaningful call stack.

`if` supports pattern-like `is` conditions and bindings propagated through
short-circuiting boolean conditions. Exhaustiveness checking is documented as
incomplete.

### Nondeterminism

`poll`, `submit`, and `repoll` implement readiness-based nondeterminism while
preserving structured ownership.

A poll server holds a pool of submitted clients. `poll` selects whichever
client becomes ready first. The active branch must finish or submit valid
descendants back to the pool. `repoll` changes handling mode while retaining the
pool.

This enables fan-in, shared linear cells, multiplexed servers, and resumable
clients. It does not provide a completely symmetric protocol where either
endpoint may spontaneously speak first. Cancellation remains cooperative.

## Standard library

The standard library is substantially implemented in Par. Rust externals are
used for primitive operations, efficient host collections, parsing engines, and
OS integration. The Par sources are embedded into `par-builtin` at compile
time.

### `@core`

The 24 core modules are:

`Bench`, `Bool`, `BoxMap`, `Byte`, `Bytes`, `Cell`, `Char`, `Data`, `Debug`,
`Float`, `Int`, `Json`, `List`, `Map`, `Nat`, `Number`, `Option`, `Ordering`,
`Stream`, `String`, `Test`, `Time`, `Try`, and `Url`.

Important components:

- `List`: mapping, flat-mapping, filtering, folds, search, copying,
  enumeration, zipping, prefix operations, extrema, stable sorting, and sums.
- `Stream`: pull-based streaming with explicit item, completion, get, and
  cancellation protocols.
- `String` and `Bytes`: composable recursive pattern parsers backed by a Rust
  finite-state parsing engine; support literals, character classes, repetition,
  concatenation, intersection, union, replacement, splitting, and streaming
  readers.
- `Json`: materialized JSON data and declarative formats for mapping,
  sequencing, literals, fields, objects, lists, unions, tags, optional values,
  and nullability.
- `Map`: ordered map with arbitrary linear values. Entry access temporarily
  removes a value; the caller must put it back or delete it.
- `BoxMap`: persistent, shareable ordered map whose values satisfy `share`.
- `Cell`: a shared linear cell/mutex implemented in Par using `poll` and
  `submit`.
- `Time`: nanosecond durations, instants, zones, zoned civil times, calendar
  arithmetic, IANA time zones, and RFC 3339 support.
- `Test`: assertion protocol used by the Par test runner.

### `@basic`

The five native basic modules are:

- `Console`: console input/output.
- `Http`: streaming client requests and an HTTP/1 server. Incoming requests
  carry a linear `Respond` choice between a regular HTTP response and a
  WebSocket upgrade.
- `Os`: paths, files, directories, standard streams, and environment access.
- `Sql`: PostgreSQL, MySQL, and SQLite through `sqlx::Any`, with linear
  connections, streaming rows, transactions, rollback cleanup, typed values,
  and temporal conversions.
- `WebSocket`: text and binary messages, independently usable linear reader
  and writer halves, native `ws`/`wss` client connections, automatic control
  frame handling, and server upgrades through `Http.Respond`.

Only `@core` is embedded on Wasm; `@basic` and its Rust external modules are
excluded.

## Examples

All 22 example modules type-check together. They cover:

- Basic programs: Hello World, Echo, Fibonacci, queues, and string handling.
- Resource-safe I/O: file copying and HTTP GET.
- Concurrent networking: an HTTP-integrated WebSocket echo server whose
  sessions run as child processes through `Cell.Share`.
- Algorithms: quicksort, deduplication, and Advent of Code.
- Nondeterminism: fan-in, source fan-in, and asynchronous reordering.
- Parsing: nested parsing and MiniGrep.
- Interactive protocols: Rock-Paper-Scissors and Sokoban.
- Services: web server, downloader pipeline, and playground chat.

The examples demonstrate functional, protocol-oriented, object-like, streaming,
and concurrent styles without separate object or async subsystems.

## Repository architecture

The main crates and directories are:

- `crates/par-core`: frontend, type system, workspace assembly, capture
  analysis, interaction-net compiler, flat transpiler, typed readback.
- `crates/par-runtime`: flat arena runtime, reducer, linking, primitive values,
  polling, external registry, and low-level readback handles.
- `crates/par-builtin`: embedded Par packages and Rust implementations of
  external definitions.
- `crates/par-doc`: type-checked HTML API documentation generator.
- `src/`: CLI, package manager, test runner, LSP, graphical playground, and
  workspace integration.
- `docs/`: English mdBook.
- `docs-ko/`: Korean mdBook.
- `examples/`: example Par package.
- `tests/`: executable Par tests.

The compilation pipeline is:

```text
Par.toml + .par files
        |
        v
package discovery and name resolution
        |
        v
lexer and high-level parser
        |
        v
desugaring to a small process IR
        |
        v
capture analysis and IR optimization
        |
        v
type validation and bidirectional checking
        |
        v
tree-shaped interaction-net packages
        |
        v
flat immutable arena representation
        |
        v
external linking
        |
        v
interaction reducer + asynchronous host externals
```

## Workspace and packages

`crates/par-core/src/workspace.rs` handles:

- Finding the nearest `Par.toml` from a file or directory.
- Recursively collecting `.par` source files.
- Multi-file modules such as `Module.part.par`.
- Local and remote dependency graphs and cycle detection.
- Imports, aliases, module canonicalization, and qualified names.
- Package, module, and item visibility.
- Built-in package injection.
- Universal package-qualified global names.
- Source overlays for the LSP and playground.
- Hover information and dot-completion queries.

Module filenames must match their declared module, case-insensitively. Directory
segments become lowercase module-path components. Multiple files may contribute
to one module, with the unsuffixed file treated as its main file.

Remote dependencies are HTTPS Git sources written without a URL scheme, such as
`github.com/user/repository`. They are shallow-cloned into the root package's
`dependencies/` tree.

Built-in aliases `@core` and `@basic` are injected into regular packages unless
the `NOSTD` environment variable is set.

## Lexer and parser

`crates/par-core/src/frontend_impl/lexer.rs` is a handwritten lexer. It tracks
precise source spans and UTF-16 columns for LSP compatibility. It supports
comments, literals, operators, and nested interpolated-template modes.

`crates/par-core/src/frontend_impl/parse.rs` uses Winnow to produce the
high-level AST in `language.rs`. The AST retains convenient source constructs:

- patterns and conditions,
- arithmetic and comparison chains,
- templates,
- lists,
- pipes,
- `try`/`catch`/`throw`,
- process blocks,
- boxes and channels,
- polling and submitting,
- implicit and explicit generic items.

## Lowering and process IR

The public facade is `crates/par-core/src/api.rs`.

Lowering translates the high-level AST into the small process IR in
`crates/par-core/src/frontend_impl/process.rs`. At this level expressions are
largely limited to:

- globals and variables,
- boxes and channels,
- primitives and externals,
- `todo`.

Processes contain protocol commands and terminal operations such as link, case,
break, begin, and loop. Operators lower to definitions in `@core/Data`,
`@core/Number`, and `@core/String`.

Conditions, pattern bindings, templates, pipes, error sugar, polling, and
implicit constructions are expanded during this phase.

Long source command sequences are processed through explicit stacks and flat
IR sequences rather than deeply recursive AST spines. Regression tests exercise
very long processes.

## Capture analysis

`crates/par-core/src/frontend_impl/captures.rs` calculates which local values
enter channel bodies, loops, polling branches, and boxed closures.

It performs fixpoint analysis across loops and classifies each variable use as a
move or copy. That result determines whether the backend emits a direct wire or
a fanout node. Source-level sharing therefore becomes an explicit runtime
operation rather than an implicit host-language clone.

## Type representation and checking

The internal `Type<S>` enum is in
`crates/par-core/src/frontend_impl/types/core.rs`. In addition to source types it
contains:

- positive and dual forms,
- inference holes and dual holes,
- recursive loop identities and ascendant sets,
- cleanup markers,
- display hints,
- a failure type for diagnostic recovery.

Type aliases are validated for legality, guarded recursive polarity,
constraints, cleanup branches, cycles, and visibility leakage.

The checker in `types/checking.rs` is bidirectional:

- Declarations provide expected types.
- Definitions without declarations are inferred.
- Linear variables are removed when consumed.
- Shareable uses are retained or compiled as copies.
- Branch contexts and obligations are merged.
- Unused droppable values synthesize `Close` commands.
- Remaining linear obligations are errors.
- Implicit type holes collect lower and upper bounds.

Definitions are checked on demand with dependency-cycle detection. Errors use a
failure type to permit further checking and richer diagnostics.

Assignability in `types/assignability.rs` is structural and guarded-recursive.
It implements primitive relationships, dual variance, either/choice width
rules, boxes, constraints, and universal/existential binders.

Totality is implemented through loop provenance and descendant tracking rather
than general theorem proving. Iterative expansion refuses an ascendant consumer;
recursive destruction must loop on a substructure. `unfounded` requests unsafe
fixpoint expansion. The interaction-net compiler independently rejects
unguarded loops.

## Interaction-net compiler

`crates/par-core/src/backend/tree/compiler.rs` compiles checked process IR into
the tree representation in `crates/par-core/src/runtime/tree/net.rs`.

Tree nodes represent positive and negative protocol operations, variables,
erasure, cleanup, duplication, packages, primitives, and externals.

Definitions, box bodies, loop bodies, and choice branches become isolated
packages containing:

- a root,
- captured context,
- internal redexes,
- the required variable-slot count.

The `max_interactions` CLI setting is a compile-time interaction/normalization
budget, not a runtime execution-step limit.

Compiler errors at this phase currently produce a generic "interaction-net
compiler bug" message instead of detailed user-facing diagnostics.

## Flat arena and artifacts

`crates/par-core/src/backend/flat/transpiler.rs` converts tree nets into the
immutable flat arena in `par-runtime`.

The flat global representation contains:

- variables,
- positive values,
- negative destructors,
- packages,
- fanouts,
- compiler-inserted `Close` nodes.

Strings, branch tables, packages, nodes, and redex arrays are interned or stored
contiguously for compactness and locality.

Compilation first produces an unlinked arena. Linking resolves external
references against the Rust registry. `par compile` serializes an unlinked
arena and the root definition map into `compiled.pvm` using Bincode. Loading an
artifact links externals again.

Artifacts currently have no explicit format or compiler-version compatibility
metadata.

## Runtime

The active runtime is `crates/par-runtime/src/flat/runtime.rs`.

Immutable global program code is separated from mutable package instances.
Each instance contains atomic one-shot variable slots. Runtime nodes are:

- `Global`: arena pointer plus package instance.
- `Linear`: dynamically allocated external/readback node.
- `Shared`: reference-counted synchronized value.
- `Empty`: temporary internal state.

Positive runtime values are break, pair, either, primitive, external function,
and captured external closure. Negative continuations are continue, pair
destruction, and choice destruction.

Fanout recursively converts a value into shared `Arc` structures. If the value
is not ready, the runtime installs a share hole and fills it later. This allows
large shared values to be reused without eager deep copies.

`Runtime::reduce` repeatedly consumes redex pairs until no work remains or an
external call is encountered. Interaction priority handles:

1. package propagation and expansion,
2. variable linking,
3. fanout and share holes,
4. expandable packages,
5. external requests/functions,
6. positive values meeting negative continuations.

Compiler-generated close nodes structurally erase a value or invoke the
designated cleanup branch.

`crates/par-runtime/src/flat/reducer.rs` is the asynchronous event loop. It
maintains external futures in `FuturesUnordered`, accepts redexes and spawned
tasks through an unbounded channel, and cooperatively alternates runtime
reduction with external progress.

## Externals and readback

Rust external functions are registered using the `external_def!` macro and
`inventory` in `crates/par-runtime/src/registry.rs`. Each definition is keyed by
package, directory path, module, and name.

The low-level readback `Handle` is effectively the Par FFI/runtime ABI. It lets
Rust code:

- send and receive,
- signal and case,
- break and continue,
- link endpoints,
- duplicate or erase,
- provide and request primitives,
- provide captured boxed closures,
- spawn concurrent protocol work.

Typed readback in `par-core` drives a handle using a checked Par type. It powers
the playground's automatically generated protocol UI. Typed readback does not
support every type form, notably arbitrary boxes, unresolved generics,
existentials, holes, or failure types.

The internal poll token is a Rust external server backed by
`FuturesUnordered`. Submitted client handles become futures, and polling selects
the first ready handle.

## Built-in implementation

`crates/par-builtin/src/builtin.rs` embeds all `.par` package sources with
`include_str!`, loads external type definitions registered through inventory,
and injects built-ins into workspaces.

There are currently 121 Par declarations implemented as Rust externals. Major
host dependencies include:

- Tokio for asynchronous I/O and runtime integration.
- Reqwest and Hyper for HTTP.
- SQLx for PostgreSQL, MySQL, and SQLite.
- Jiff for date/time and time zones.
- Serde JSON for JSON materialization.
- `BTreeMap` and `im::OrdMap` for linear and persistent maps.

External implementations assume that the Par type declaration and runtime
protocol are obeyed. Protocol mismatches commonly use `panic!`, `expect`, or
`unreachable!`, so this boundary is trusted code.

## CLI, tests, LSP, docs, and playground

The CLI in `src/main.rs` provides:

- `new`
- `playground`
- `add`
- `run`
- `check`
- `doc`
- `compile`
- `run-vm`
- `lsp`
- `update`
- `test`

The test runner in `src/test_runner.rs` discovers:

- definitions named `Test...` with a type assignable to `[Test] !`,
- definitions named `Run...` with type `!`.

Tests run through the same compiler, linker, reducer, and external registry as
normal programs. Test assertions are communicated through the `@core/Test`
protocol.

The LSP provides:

- workspace diagnostics,
- hover with inferred types and documentation,
- document symbols,
- code lenses,
- dot completion,
- go-to-declaration and go-to-definition for globals,
- access to embedded built-in source files.

It recompiles complete workspaces with in-memory source overlays. It preserves
the last successfully checked workspace after a failed compile so completion
and navigation can continue.

The HTML docs generator in `par-doc` discovers and type-checks a workspace,
builds a package/module/item model, renders types with links, and distinguishes
root, direct dependency, indirect dependency, and built-in packages.

The graphical playground edits and compiles packages, displays lowered code,
runs definitions, and uses typed readback to generate controls for values and
protocol choices. Native builds support package overlays; Wasm uses embedded or
synthetic sources.

## Validation baseline

At the commit recorded above, these checks passed:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo check --no-default-features
cargo check --target wasm32-unknown-unknown
cargo test --workspace
cargo run --quiet -- test --package tests
mdbook test docs
mdbook build docs
mdbook test docs-ko
mdbook build docs-ko
```

Observed results:

- 123 `par-core` tests passed.
- 12 root integration/tooling tests passed.
- 143 total Rust tests passed, including eight WebSocket and HTTP-upgrade
  lifecycle tests.
- All 22 example modules type-checked.
- All 78 executable Par tests passed, including SQL, cleanup, parsing,
  collections, streams, templates, numeric operations, and time zones.
- Both books passed mdBook tests and link builds.
- Native, headless, and Wasm checks succeeded.

Native default builds emitted two future-compatibility warnings for unsuffixed
`f32` literals in playground UI code. The Wasm build emitted many unused-code
warnings because native CLI paths are compiled out.

## Strengths

- The design is coherent: features arise from dual logical connectives instead
  of independent subsystems.
- Session protocols and ownership use the same type system.
- Deadlock avoidance and cleanup are structural rather than convention-based.
- Totality and productivity cover common finite data and long-lived services.
- Explicit errors fit concurrent evaluation better than stack unwinding.
- The standard library strongly dogfoods Par.
- Streams, parsers, maps, SQL transactions, HTTP bodies, and cells demonstrate
  real resource-safety use cases.
- The compiler preserves the logical model down to interaction rules.
- Workspace tooling understands packages, imports, visibility, overlays,
  source spans, hover, and completion.
- End-to-end tests cover significant real behavior rather than only parser
  snapshots.

## Known limitations and risks

### Language-level

- The execution and ownership model has a steep learning curve.
- The totality checker is conservative and needs `unfounded` for some valid
  divide-and-conquer and nested parsing algorithms.
- There is intentionally no dependent typing, metaprogramming, macro system, or
  higher-kinded type system.
- Global definitions and aliases cannot participate in dependency cycles.
- Implicit generic inference is deliberately local.
- Auto-cleanup requires a statically designated branch and cannot recursively
  consume an unbounded iterative structure.
- `if` exhaustiveness checking is incomplete.
- Readiness nondeterminism cannot directly express fully symmetric
  either-endpoint-first sessions.
- Cancellation is cooperative.
- Integer division by zero currently returns zero, while floating-point
  division uses IEEE behavior.
- Structural typing provides less nominal separation by default.

### Runtime and compiler

- Net rewriting is single-reducer rather than multicore parallel.
- The source-language "no panics" property does not cover compiler/runtime/FFI
  bugs; trusted Rust contains invariant panics.
- There are latent `todo!` paths in arena defaults and legacy tree-request
  transpilation. Current tests do not reach them.
- Instance leak checking is disabled because cancellation can leave states that
  trigger it; see the comment referencing issue 165 in the flat runtime.
- Interaction-net backend errors are generic bug reports.
- No systematic performance benchmark suite establishes runtime
  characteristics.
- The compiler still constructs a tree net before converting it to the flat
  arena, which is clear architecturally but adds complexity and legacy surface.

### Packages and artifacts

- Remote dependencies have no versions, commit pins, checksums, or lockfile.
  Shallow clones follow the remote default branch, so builds are not
  reproducible.
- `par update` removes the managed dependency tree before refetching it.
- `.pvm` artifacts have no explicit format/version compatibility metadata.

### CLI and release process

- `-f`/`--flag` options are declared but never consumed.
- Some `run`, `compile`, and `run-vm` errors print messages without returning a
  failing process exit code.
- `par new` prints its error twice.
- `par compile` always writes `compiled.pvm` in the current directory.
- The release workflow appears stale: Cargo builds `par`, while the workflow
  expects `par-lang`, and it invokes `check` using an obsolete file-path form.
- `docs/src/getting_started.md` says there are no prebuilt releases; do not
  infer actual release availability from the workflow alone.

### LSP and documentation

- The LSP advertises execution support, but `run_in_playground` only logs that
  running is unsupported.
- Go-to-definition/declaration does not handle local variables.
- Document-close handling is unfinished.
- Some symbol-range behavior has a documented bug.
- The book is broad but uneven: a few chapters or index files are placeholders
  or very sparse.

### Test distribution

- Test coverage is concentrated in `par-core` and root integration tests.
- `par-runtime`, `par-builtin`, and `par-doc` have no independent unit tests,
  though they receive substantial end-to-end coverage through examples and Par
  tests.
- The executable test suite can produce noisy debug output even when tests
  pass, notably repeated "Round-trip did not work" messages from binary tests.

## Guidance for future coding agents

Before changing compiler behavior, trace the feature through all relevant
layers:

1. Lexer/parser and high-level AST.
2. Lowering into process IR.
3. Capture analysis.
4. Type validation, checking, and assignability.
5. Tree interaction-net compilation.
6. Flat transpilation.
7. Runtime interaction or external readback.
8. Built-in Par declarations and Rust external registrations.
9. CLI/LSP/playground surfaces.
10. Rust tests, Par tests, examples, and book snippets.

For ownership-related changes, verify both the static and dynamic halves:

- whether the type satisfies `drop` or `share`,
- whether checking inserts `Close` or capture usage correctly,
- whether the compiler emits erasure/fanout,
- whether the runtime closes or shares the corresponding value shape.

For recursive changes, inspect both type-level ascendant tracking and backend
unguarded-loop detection. Do not assume acceptance by one layer implies the
other layer will compile it.

For externals, keep the Par declaration and Rust protocol implementation in
lockstep. The Rust side is trusted and often panics on a mismatched sequence.

For package/LSP work, use workspace APIs rather than parsing a file in
isolation. Built-ins, dependency aliases, visibility, multifile modules,
universal names, and source overlays all affect results.

After a meaningful language or runtime change, the minimum broad validation is:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo run --quiet -- test --package tests
mdbook test docs
```

Also run the Wasm check for changes touching runtime, built-ins, playground, or
conditional compilation:

```sh
cargo check --target wasm32-unknown-unknown
```

## Recommended source entry points

- Language rationale: `docs/src/introduction.md`
- Type summary: `docs/src/big_table.md`
- Constraints and cleanup: `docs/src/types/constraints.md` and
  `docs/src/types/auto_cleanup.md`
- Process syntax: `docs/src/process_syntax.md`
- Nondeterminism: `docs/src/nondeterminism/`
- Compiler API: `crates/par-core/src/api.rs`
- High-level AST/lowering: `crates/par-core/src/frontend_impl/language.rs`
- Process IR: `crates/par-core/src/frontend_impl/process.rs`
- Type checker: `crates/par-core/src/frontend_impl/types/checking.rs`
- Type representation: `crates/par-core/src/frontend_impl/types/core.rs`
- Assignability: `crates/par-core/src/frontend_impl/types/assignability.rs`
- Workspace: `crates/par-core/src/workspace.rs`
- Tree compiler: `crates/par-core/src/backend/tree/compiler.rs`
- Flat transpiler: `crates/par-core/src/backend/flat/transpiler.rs`
- Flat runtime: `crates/par-runtime/src/flat/runtime.rs`
- Async reducer: `crates/par-runtime/src/flat/reducer.rs`
- Readback/FFI: `crates/par-runtime/src/flat/readback.rs` and
  `crates/par-runtime/src/readback.rs`
- External registry: `crates/par-runtime/src/registry.rs`
- Built-in injection: `crates/par-builtin/src/builtin.rs`
- Built-in packages: `crates/par-builtin/packages/core/src/` and
  `crates/par-builtin/packages/basic/src/`
- CLI: `src/main.rs`
- Test runner: `src/test_runner.rs`
- LSP: `src/language_server/`
- Playground: `src/playground/`
- Documentation generator: `crates/par-doc/src/`

## Bottom line

Par's language design is ahead of its product maturity. The compiler,
interaction-net runtime, standard library, SQL/HTTP/OS integrations, tests,
documentation generator, playground, and LSP form a real working system.

The highest-leverage work is likely to be hardening rather than adding new
language concepts: improve the runtime/FFI boundary, make dependencies and
artifacts reproducible, repair release automation and exit semantics, finish
LSP behavior, improve backend diagnostics, and measure runtime performance.
