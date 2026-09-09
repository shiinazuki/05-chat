# CLAUDE.md

Project conventions for AI coding agents (Claude Code, Cursor, Codex, etc.) and human developers.

This project enforces several **counter-intuitive** rules. Default Rust habits will directly break CI. Read this document before making changes.

## Primary Entry Point: Always Use `just`

```bash
just            # List all available recipes
just check      # Fast compilation check (cargo check)
just test       # Run full test suite (cargo-nextest + doctests)
just lint       # Formatting, TOML, clippy, typos, and doc checks (equivalent to the CI lint job)
just ci         # Full local gate: lint + test + audit (this is what the pre-push hook runs)
```

**Run `just lint` at least once after modifying code.** Do not rely solely on `cargo build`—CI gates on clippy and rustdoc warnings, not just compilation success.

## 5 Critical Gotchas (Do Not Violate)

### 1. Formatting MUST use nightly via `just fmt`

```bash
just fmt        # NEVER run raw `cargo fmt` or `cargo +nightly fmt` directly
```

`rustfmt.toml` relies on **unstable options** (such as `imports_granularity`, `group_imports`, `wrap_comments`). Stable `rustfmt` **silently ignores** them—no error, no formatting—so a local check passes while CI fails.

Do not hardcode `cargo +nightly fmt` either: if `rust-toolchain.toml` pins a dated channel (e.g. `nightly-2026-08-18`), plain `+nightly` resolves to a **different** toolchain that may format differently. `just fmt` derives the exact toolchain from the channel (see `fmt_toolchain` in `justfile`).

### 2. Zero Warnings Policy in CI

CI enforces `-D warnings` for both `cargo clippy` and rustdoc (`RUSTDOCFLAGS="-D warnings"`). Any warning fails the build, including broken intra-doc links, bare URLs, and invalid HTML in doc comments.

### 3. Never Suppress Lints to Bypass CI

When clippy complains, **fix the underlying code**. If suppression is genuinely unavoidable:

```rust
#[expect(clippy::needless_pass_by_value, reason = "required by trait signature")]
```

- Always use `#[expect]`, never `#[allow]`—`clippy::allow_attributes` is a warning, and warnings fail CI. `#[expect]` also flags itself for removal once the lint stops triggering.
- **`reason = "..."` is mandatory.**
- `unsafe_code = "forbid"` is set workspace-wide and cannot be overridden by an attribute. If unsafe code is truly required, ask a human maintainer to downgrade it to `deny`.

### 4. Compiler Version is Pinned by `rust-toolchain.toml`

Do not override the toolchain with `rustup override set` or the `RUSTUP_TOOLCHAIN` environment variable. Both take silent precedence over `rust-toolchain.toml`; `just doctor` checks for exactly this.

If the compiler itself crashes (`error: internal compiler error`, leaving `rustc-ice-*.txt`), run `just ice` before changing any code—it reports which compiler build crashed and where.

### 5. Never Use `println!` for Standard Output

Use the `print_line()` helper in `src/main.rs`. `println!` panics when a downstream pipe closes early (e.g. `app | head -n 1`). Diagnostics go to `stderr`, business output goes to `stdout`—never mix them.

## Code Organization & Visibility

- **Keep all business logic in `src/lib.rs` and its modules.** `src/main.rs` is reserved for CLI argument parsing, logging initialization, and top-level error handling (<100 lines).
- Code in `main.rs` is unreachable from integration tests (`tests/`), benchmarks, and doctests.
- `pub` is a public contract. Crate-internal utilities must use `pub(crate)` (`unreachable_pub` is enforced).

## Error Handling Strategy

- **Library side (`src/error.rs`)**: define domain-specific, matchable error enums with `thiserror`.
- **Application side (`src/main.rs`)**: use `anyhow::Result` with `.context()` for readable error stacks.
- **Never expose `anyhow::Error` from a public library API**—callers lose the ability to match on specific variants.
- New error variants can be added freely; the enum is `#[non_exhaustive]`.
- Document failure cases of public `Result`-returning functions under an `# Errors` section. This is a house convention, not a CI gate—`missing_errors_doc` is set to `allow`.

## Async Rules: Zero Blocking I/O

`clippy.toml` sets `disallowed-types` and `disallowed-methods` against blocking calls such as `std::fs::*` and `std::process::Command`. Use `tokio::fs::*` and `tokio::process::Command` instead.

The ban does not distinguish async from sync context—tests and `build.rs` are caught too. Where synchronous I/O is genuinely required (e.g. reading config before the runtime starts):

```rust
#[expect(clippy::disallowed_methods, reason = "one-time startup config read before runtime launch")]
```

## Dependency Management

- Declare versions once in the root `Cargo.toml` under `[workspace.dependencies]`; member manifests only write `dep_name = { workspace = true }`. The two tables sit next to each other, and once a version is listed in the first, a bare `cargo add <dep>` writes the inheriting form into the second on its own. Do not pass a version on the command line — `cargo add <dep>@1.2` writes a concrete version instead, even when it matches the catalog entry.
- **Run `just audit` (`cargo-deny`) after touching dependencies.** It checks advisories, licenses, banned crates, and wildcard versions.
- Never edit `Cargo.lock` by hand. CI and Docker builds run with `--locked`.
- Check whether the standard library already covers it before adding a dependency—`[bans.std-replacements]` in `deny.toml` rejects crates that `std` has absorbed (e.g. `lazy_static` vs `std::sync::LazyLock`).

## Testing & Context Control

- Filter tests while debugging to keep output readable:
  ```bash
  cargo nextest run <test_filter>
  ```
- Run `just test` before submitting changes (nextest plus doctests).
- nextest runs each test in its own process, in parallel. For tests sharing a resource (port, database, file), declare a serialization group under `[test-groups]` in `.config/nextest.toml` rather than adding a `Mutex` inside the test code.
- Integration tests (`tests/`) can only reach `pub` APIs.
- `unwrap()`, `expect()`, and `panic!()` are allowed inside tests—`clippy.toml` permits them there.

## Git & Commit Workflow

Run `just hooks` to install the hooks. **Never bypass them with `--no-verify`.**

| Hook | Executed Tasks |
| --- | --- |
| `pre-commit` | Change-scoped fast checks: `rustfmt`, `clippy`, `taplo`, `cargo-deny`, `typos`, private-key detection |
| `commit-msg` | Validates Conventional Commits format |
| `pre-push` | Runs the full `just ci` pipeline |

Commit messages must follow Conventional Commits:

```text
feat(cli): add --json output support
fix: prevent panic when stdout pipe closes early
refactor!: change greet return type to Result      # ! denotes breaking change
```

Allowed types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`, `revert`, `style`, `test`.
CHANGELOG entries and `cargo-release` version bumps are derived from these types via `cliff.toml`.

## Prohibited Actions

- Do NOT create unnecessary configuration files.
- Do NOT weaken lints in `Cargo.toml` (`[workspace.lints]`), `clippy.toml`, `rustfmt.toml`, or `deny.toml` to make CI pass.
- Do NOT put domain logic in `src/main.rs`.
- Do NOT delete CI jobs to shorten pipeline duration.
- When uncertain, ask instead of guessing or rewriting configuration.
