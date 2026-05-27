# Changelog

## 0.1.4

- Disable PyPI digital attestations to avoid transient Sigstore/Rekor upload failures while keeping Trusted Publishing.
- Add a release binary build timeout so stuck hosted runners fail quickly.

## 0.1.3

- Limit PyPI publishing to Python wheels and source distribution artifacts.
- Checkout sources before creating GitHub Releases from annotated tag notes.

## 0.1.2

- Move macOS Intel release jobs to GitHub's current `macos-15-intel` runner label.
- Publish a clean patch release after the `v0.1.1` release workflow was blocked waiting for unavailable `macos-13` runners.

## 0.1.1

- Add dashboard APIs, Vite React dashboard, release binary assets, and install scripts.
- Fix macOS PyO3 test linking by enabling `extension-module` only for maturin builds.
- Include license files in source distributions for PyPI uploads.

## 0.1.0

- Initial Renderacre controller, worker, OpenJD, and Python package scaffold.
