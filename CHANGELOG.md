# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-07

### Added

- Netted settlement: collateral deposits, atomic obligation batches checked
  for coverage, and a `settle` that cannot fail for lack of funds.
- Operator controls that never trap funds: pause, token allowlist, per-token
  minimum obligation amounts, member suspension, and a two-step operator
  handover.
- `admit_many`, `withdraw_all`, `open_positions`, `reference_used` and a
  permissionless `extend_ttl`.
- Events for every state change, including each member's applied net at
  settlement.
- Contract metadata naming the source repository.
- Randomised invariant tests (conservation, zero-sum positions, coverage,
  atomicity) and a test holding the largest window within 75% of Soroban's
  per-transaction limits.
- `scripts/testnet-demo.sh`, CI checks for WASM size, TypeScript bindings and
  cargo-deny, and releases that publish the WASM with its SHA-256.

### Fixed

- One member could fill a window's positions with dust obligations and lock
  everyone else out until settlement; a per-member position quota now limits
  each member to 16 new positions per window by default.
- Windows had no default bound, so if the operator disappeared, collateral
  committed to the open window stayed locked. Windows now close permissionlessly
  after 7 days by default, and the bound cannot be unset.

[Unreleased]: https://github.com/SetOff-Org/setoff-contracts/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/SetOff-Org/setoff-contracts/releases/tag/v0.1.0
