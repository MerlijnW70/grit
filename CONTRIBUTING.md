# Contributing to Grit

## Layout

Grit builds against `simdr` by path, so clone it beside this repository:

```sh
git clone https://github.com/MerlijnW70/grit
git clone https://github.com/MerlijnW70/simdr
git -C simdr checkout <SIMDR_REV>
```

`SIMDR_REV` is the commit pinned at the top of `.github/workflows/ci.yml`. CI builds against exactly that commit.

## What CI runs

Every pull request runs these. Run them from the `grit` directory first.

```sh
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --locked --no-default-features --all-targets
RUSTFLAGS="-D warnings" cargo clippy --locked --all-targets --features faults
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
cargo +1.88 check --locked --no-default-features --all-targets
```

`1.88` is `rust-version` in `Cargo.toml`. The default features open a window, so on Linux the second clippy and the doc run need the X11 and Wayland development headers (the list is in the `headers` step of `ci.yml`).

The suite runs on lavapipe, the software Vulkan driver, at subgroup widths 4, 8 and 16. Install `mesa-vulkan-drivers`, point `VK_ICD_FILENAMES` at `lvp_icd*.json`, and run once per `LP_NATIVE_VECTOR_WIDTH` of 128, 256 and 512:

```sh
LP_NATIVE_VECTOR_WIDTH=128 GRIT_PATIENCE_SECS=300 \
  cargo test --release --locked --no-default-features --features deep,faults --no-fail-fast -- --nocapture
```

CI also counts the `SKIPPED` lines and fails on a different count (16, 9 and 6 for widths 4, 8 and 16).

## Rules

- A change adds no comments and no prose to code. Behaviour is stated by a test.
- A failing test is fixed at its root cause. A test changes only when the test was wrong.
- Budget and timing behaviour is tested by evidence (the `faults` hooks, counters), never by wall time.
- Every change goes through a pull request and green CI.

## Sign-off

Every commit carries a Developer Certificate of Origin sign-off (`git commit -s`). The certificate is at https://developercertificate.org/.

## License

Unless you state otherwise, a contribution is licensed as `MIT OR Apache-2.0`, without any additional terms or conditions.

## Expectations

There is no promise of payment, of a response time or of support.
