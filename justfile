set shell := ["powershell", "-NoProfile", "-Command"]

fmt:
    cargo fmt --all -- --check

test:
    cargo test --workspace

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

wheel:
    python -m maturin build --release -o target\wheels

e2e:
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\e2e_smoke.ps1

bins:
    cargo build --release -p renderacre-controller -p renderacre-worker

preflight: fmt clippy test e2e wheel bins
