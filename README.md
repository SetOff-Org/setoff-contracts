<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
    <img src="assets/logo.svg" alt="SetOff" height="64">
  </picture>
</p>

<p align="center"><b>Soroban contracts for netted settlement on Stellar. Obligations accrue all window, only net positions move, and settlement cannot fail.</b></p>

<p align="center">
  <a href="https://github.com/SetOff-Org/setoff-contracts/actions/workflows/ci.yml"><img src="https://github.com/SetOff-Org/setoff-contracts/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License"></a>
</p>

---

Anchors, payment providers and agent platforms on Stellar owe each other all
day. Settling every obligation gross means each of them pre-funds its entire
outflow. The `settlement` contract settles **net**: members record what they
owe during a window, and at settlement each member's balance moves by its
net position only. No tokens move between members at all.

The safety rule: **after every batch of obligations, each debtor's net debit
must be covered by its deposited balance.** Members can owe far more than they
hold, as long as it nets out, and `settle` can never fail for lack of funds.

## Deployed on testnet

Contract [`CCV7S3FE…WNGU`](https://stellar.expert/explorer/testnet/contract/CCV7S3FEDA5TXR6IKS3R2PE3UZVET2U76SCTGBI3WDE6EKINKDV4WNGU),
using native XLM as the settlement token:

| Step | Transaction |
|---|---|
| A deposits 10 XLM collateral | [`7740672b…95d1`](https://stellar.expert/explorer/testnet/tx/7740672bfaed30c11de4a35b0e0315992f02fd5629a18aea5e43c7114f0595d1) |
| A owes B 10 XLM | [`48ab112e…782f`](https://stellar.expert/explorer/testnet/tx/48ab112ed566a3ddcea9340ac2d99871b4fcd01c726ff127ccc8e96fb69f782f) |
| B owes C 10 XLM, with **no collateral**: B is already owed 10 this window | [`9860bff1…73dc`](https://stellar.expert/explorer/testnet/tx/9860bff1c19695dd4269b01fde756de738b8a725b181d6f60ebb234a941573dc) |
| C owes A 10 XLM | [`ba7d2035…885c`](https://stellar.expert/explorer/testnet/tx/ba7d2035e08aecbfe3e74f9538619652dc005fae6ef2f589a26b07193840885c) |
| Settle: 30 XLM gross, every position 0, nothing moves | [`579ad1c6…0f9a`](https://stellar.expert/explorer/testnet/tx/579ad1c6f6db8e1c2c1f4ab267ed8bd7b4e4ebaddfe1390dc3689b4fb6010f9a) |

## Interface

| Function | Who | What |
|---|---|---|
| `__constructor(admin)` | deployer | Sets the operator |
| `admit(member)` | operator | Admits a member |
| `deposit(member, token, amount)` | member | Moves collateral into the contract |
| `withdraw(member, token, amount)` | member | Returns collateral not committed to the open window |
| `submit(obligations)` | every debtor in the batch | Records up to 32 obligations atomically; rejects the batch if any debtor's net debit is uncovered |
| `settle()` | operator | Applies every net position to balances and opens the next window |
| `balance`, `position`, `available`, `gross`, `window`, `is_member`, `admin` | anyone | Views |

Events: `Admitted`, `Deposited`, `Withdrawn`, `Obligated` (debtor and
creditor as topics) and `Settled` (window as topic). Settlement receipts can be
proven to third parties with [Externalize](https://github.com/Externalize-Labs/externalize).

Limits: 32 obligations per batch and 64 (member, token) positions per window,
which keeps `settle` well inside Soroban's resource limits. The WASM is about
11 KB.

## Build and test

```sh
cargo test
cd contracts/settlement && stellar contract build
```

The tests cover the zero-collateral cycle, bilateral netting, uncovered
debits, batch atomicity, withdrawal guards, debtor and operator authorization,
duplicate references, and window limits.

## Related

- [setoff-engine](https://github.com/SetOff-Org/setoff-engine): the off-chain
  netting algorithm and its reference vectors.
- [setoff-clearing](https://github.com/SetOff-Org/setoff-clearing): the clearing
  service and `setoff` CLI.

## Status

Not audited. See [SECURITY.md](SECURITY.md). Known gaps (default handling and
member removal, permissionless settlement on a schedule, multi-token windows
beyond 64 positions) are tracked as issues.

## License

[Apache-2.0](LICENSE)
