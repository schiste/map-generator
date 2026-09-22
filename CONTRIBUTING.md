# Contributing

Thanks for helping! A few ground rules keep the output reproducible:

1. `cargo fmt --all && cargo clippy --workspace --all-targets && cargo test --workspace`
   must pass. CI runs these commands on Linux, macOS, and Windows.
2. **Never introduce nondeterminism** into `mapgen-core`: no `HashMap` iteration
   in output paths, no timestamps, no locale-dependent formatting.
3. If you intentionally change the SVG output, regenerate the golden files and
   review the diff:
   ```sh
   UPDATE_GOLDEN=1 cargo test -p mapgen-data --test golden
   git diff crates/mapgen-data/tests/fixtures
   ```
4. Do not commit datasets or maps generated from GADM (see `docs/data-sources.md`).
   Test fixtures must be hand-made or public domain.

Contributions are accepted under the MIT License.
