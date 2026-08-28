# Contributing to hugit

## Workflow

1. Fork the repository and create a branch from `main`.
2. Make your changes. Keep commits focused and atomic.
3. Build and test locally:
   ```sh
   cargo fmt --all --check
   cargo build --workspace --locked
   cargo test --workspace --locked
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo deny check
   ```
4. Sign off your commits with `-s` (DCO — Developer Certificate of Origin):
   ```sh
   git commit -s -m "your message"
   ```
5. Open a pull request against `main`. The CI gate must be green before merge.

## DCO

By signing off your commit you certify that you have the right to submit the
contribution under the project's Apache-2.0 license. See
<https://developercertificate.org/> for the full text.

## Code style

Follow the existing patterns. Run `cargo fmt --all` before committing.

## Questions

Open a discussion or issue on GitHub for design questions before writing code.
