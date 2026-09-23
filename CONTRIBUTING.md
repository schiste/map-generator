# Contributing

Thanks for helping! A few ground rules keep the output reproducible:

1. `cargo fmt --all && cargo clippy --workspace --all-targets && cargo test --workspace`
   must pass. CI runs these commands on Linux, macOS, and Windows.
   For the WebAssembly crate, see `crates/mapgen-wasm/README.md` (wasm-pack, Node and
   headless-Chrome tests, TypeScript check, native parity).
2. **Never introduce nondeterminism** into `mapgen-core`: no `HashMap` iteration
   in output paths, no timestamps, no locale-dependent formatting, and no `f64::sin`/`cos`/… (use `crate::math`, backed by `libm`).
3. If you intentionally change the SVG output, regenerate the golden files and
   review the diff:
   ```sh
   UPDATE_GOLDEN=1 cargo test -p mapgen-data --test golden
   git diff crates/mapgen-data/tests/fixtures
   ```
4. If the change affects rendering, run `scripts/build-examples.sh` (after
   `scripts/fetch-data.sh ne-geojson`) and commit the updated `docs/examples`.
5. Only public-domain or openly licensed data. Do not add support for sources with
   non-commercial or no-redistribution terms (see `docs/data-sources.md`). Do not commit
   datasets; test fixtures must be hand-made or public domain.

Contributions are accepted under the MIT License.
