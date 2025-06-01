# Contributing to Textra

First off, thank you for considering contributing to Textra! It's people like you that make Textra such a great tool.

## Development Setup

Ensure you have Rust installed. You can get it from [rustup.rs](https://rustup.rs/).

```bash
# Clone the repository
git clone https://github.com/username/textra.git # Replace with actual repository URL
cd textra

# Build the project
cargo build

# Run tests
cargo test
```

## CI/CD Pipeline (Conceptual Outline)

We aim to use GitHub Actions for our CI/CD pipeline. Below is a conceptual outline of the workflow.

File: `.github/workflows/rust.yml`

```yaml
name: Rust CI

on: [push, pull_request]

env:
  CARGO_TERM_COLOR: always

jobs:
  build_and_test:
    name: Build and Test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Set up Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          profile: minimal # Or 'default' if more components are needed
          override: true

      - name: Cache dependencies
        uses: Swatinem/rust-cache@v2
        # Caches ~/.cargo and target/debug/deps

      - name: Check formatting
        run: cargo fmt --all -- --check

      - name: Lint code
        run: cargo clippy --all-targets --all-features -- -D warnings # Fail on warnings

      - name: Build
        run: cargo build --verbose

      - name: Run tests
        run: cargo test --verbose --all-features

  # Optional: Build for Windows (if your primary dev/test is Linux)
  build_and_test_windows:
    name: Build and Test (Windows)
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v3

      - name: Set up Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          profile: minimal
          override: true
      
      - name: Cache dependencies
        uses: Swatinem/rust-cache@v2

      - name: Build (Windows)
        run: cargo build --verbose

      - name: Run tests (Windows)
        run: cargo test --verbose --all-features
```

### Workflow Steps Explanation:

1.  **`on: [push, pull_request]`**: Triggers the workflow on every push to any branch and on any pull request.
2.  **`env: CARGO_TERM_COLOR: always`**: Ensures colored output from Cargo commands in the logs.
3.  **`jobs: build_and_test`**: Defines a job named `build_and_test`.
    *   **`runs-on: ubuntu-latest`**: Specifies that this job will run on the latest version of Ubuntu provided by GitHub Actions.
    *   **`steps:`**: A sequence of tasks to be executed.
        *   **`actions/checkout@v3`**: Checks out your repository's code into the runner.
        *   **`actions-rs/toolchain@v1`**: Sets up the Rust toolchain.
            *   `toolchain: stable`: Uses the stable Rust toolchain.
            *   `profile: minimal`: Installs a minimal set of components.
            *   `override: true`: Ensures this version of Rust is used.
        *   **`Swatinem/rust-cache@v2`**: Caches Cargo dependencies to speed up subsequent builds.
        *   **`cargo fmt --all -- --check`**: Checks if the code is formatted according to Rustfmt style. Fails if not.
        *   **`cargo clippy --all-targets --all-features -- -D warnings`**: Runs Clippy, a linter for Rust, to catch common mistakes and improve code quality. `-D warnings` makes Clippy treat warnings as errors, failing the build.
        *   **`cargo build --verbose`**: Compiles the project. `--verbose` provides more detailed output.
        *   **`cargo test --verbose --all-features`**: Runs all tests in the project. `--all-features` ensures tests for all feature flags are run.
4.  **`jobs: build_and_test_windows`**: (Optional) A similar job that runs on `windows-latest` to ensure compatibility.

This outline provides a basic structure. It can be expanded with steps for creating releases, publishing to crates.io, etc.