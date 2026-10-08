# Contributing

The settlement contract holds members' collateral, so changes here get the
closest review of any SetOff repository. It takes part in the
[Stellar Wave](https://www.drips.network/wave/stellar) program; Wave issues are
labeled with their complexity. The ground rules for every SetOff repository are
in the [organization guide](https://github.com/SetOff-Org/.github/blob/main/CONTRIBUTING.md).

## Setup

```sh
git clone https://github.com/SetOff-Org/setoff-contracts && cd setoff-contracts
cargo test
```

`rust-toolchain.toml` pins the toolchain. Building the WASM needs
[stellar-cli](https://developers.stellar.org/docs/tools/cli) 25.2 or later.

## Before you open a PR

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test
stellar contract build
```

CI also checks that the WASM stays within a 64 KiB budget, and runs cargo-deny.

## Rules specific to the contract

- **Invariants first.** `balance + min(position, 0) >= 0` for every member,
  positions sum to zero, and no operator action can trap a member's available
  funds. `src/invariants.rs` checks them under random sequences of operations;
  a change that needs a new invariant adds it there.
- **Resource limits.** `src/resources.rs` settles the largest window the
  contract allows and asserts it fits one transaction under testnet's limits.
  Keep it passing; if a change makes windows more expensive, lower the limits
  rather than the margin.
- **Errors are append-only.** Existing `Error` codes never change meaning; new
  ones go at the end.
- **Try it on testnet.** [`scripts/testnet-demo.sh`](scripts/testnet-demo.sh)
  deploys a fresh contract and runs a full cycle.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org): `feat(contract): …`,
`fix(settle): …`, `docs: …`.
