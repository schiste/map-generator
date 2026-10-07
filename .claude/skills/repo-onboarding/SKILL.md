---
name: repo-onboarding
description: Use when starting work in an unfamiliar repository, when the task asks for repo overview, setup, architecture, entrypoints, test commands, or where to begin. Skip for narrow file-scoped edits once the relevant paths are already known.
---

# Repo Onboarding: map-generator

## When to Use

- Load this skill first when the repository is unfamiliar or the request is broad.
- Recommended when: first task in repo, repo overview, setup or run instructions, architecture or entrypoints, where should I start, broad debugging or feature-localization request.
- Skip when: known file-scoped edit, follow-up inside already identified area, task already localized to concrete files.
- Use `.codex/skills/aethyme/SKILL.md` or `.claude/skills/aethyme/SKILL.md` for Aethyme's short operating contract after orientation; load its `references/` files only when needed.

## Repo Identity

- Kind: `repository`
- Languages: `rust, typescript`
- Package manager: `cargo`
- Key manifests: `Cargo.toml, Procfile, crates/mapgen-cli/Cargo.toml, crates/mapgen-core/Cargo.toml, crates/mapgen-data/Cargo.toml, crates/mapgen-server/Cargo.toml, crates/mapgen-spec/Cargo.toml, crates/mapgen-wasm/Cargo.toml, crates/mapgen-wasm/package.json`

## Workspaces

- `.` (primary; cargo; manifest `Cargo.toml`; high confidence)
- `crates/mapgen-wasm` (supporting; npm; manifest `crates/mapgen-wasm/package.json`; high confidence)

## Start Here

- `dev`: `cargo run`
- `fast_test`: `cargo test --workspace`
- `build`: `cargo build --workspace`

## Supporting Commands

- `cargo run` (dev; medium confidence from `Cargo.toml`)
- `./target/release/mapgen-server` (dev; medium confidence from `Procfile:web`)
- `cargo test` (fast_test; high confidence from `Cargo.toml`)
- `cargo test --workspace` (fast_test; high confidence from `Cargo.toml`)
  Workspace: `.`
- `cargo build` (build; high confidence from `Cargo.toml`)

## Entrypoints

- `app`: `Procfile:web` (Procfile process entrypoint; high confidence)
- `cli`: `crates/mapgen-cli/src/main.rs` (tracked Rust binary entrypoint in `.`; high confidence)
- `test`: `tests` (conventional test root; medium confidence)

## Additional Entrypoints

- `Procfile:web` (process; role=app; Procfile process entrypoint; high confidence)
- `crates/mapgen-cli/src/main.rs` (file; role=cli; tracked Rust binary entrypoint in `.`; high confidence)
  Executable: `mapgen`
- `crates/mapgen-server/src/main.rs` (file; role=cli; tracked Rust binary entrypoint in `.`; high confidence)
  Executable: `mapgen-server`
- `tests` (directory; role=test; conventional test root; medium confidence)

## Repo Map

- `.github` (automation; automation and CI configuration; high confidence)
- `docs` (docs; documentation area; high confidence)
- `scripts` (tooling; developer tooling or scripts; high confidence)
- `tests` (tests; conventional test directory; high confidence)

## Aethyme Recipes

- `aethyme explore --repo "$PWD" --request "<task>" --format answer-json`
  Purpose: Broad repository orientation for a user request
- `aethyme repo inspect "$PWD" --mode brief --json-output`
  Purpose: Quick deterministic repo summary
- `aethyme graph callers "$PWD" "<symbol-or-file>" --json-output`
  Purpose: Trace likely impact before editing

## Generated and Dangerous Paths

- Generated/vendor `.aethyme/generated`: tracked generated or vendored surface; verify ownership before editing
- Sensitive `.github/workflows`: repository automation; changes can affect publication or shared CI

## Freshness

- Source digest: `9d585c352194147824bc58c079ba104409fa5271d4cab3e0f4363992fb631475`
- Tracked source files: `131`
- Overrides applied: `False`
- Sections generated: `repo, workspaces, primary_workspace, commands, areas, entrypoints, caution_zones, generated_paths, dangerous_paths, navigation_recipes, summon, freshness`
