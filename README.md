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

Contract [`CA7VZ3TF…CFFP`](https://stellar.expert/explorer/testnet/contract/CA7VZ3TFZMOZAK5CKLSEAG3CS3VMFCB6FXY2W6R4XUY53JAYUZFDCFFP),
using native XLM as the settlement token. [`scripts/testnet-demo.sh`](scripts/testnet-demo.sh)
deploys a fresh copy and replays this:

| Step | Transaction |
|---|---|
| Allow native XLM | [`1e6db7cf…6346`](https://stellar.expert/explorer/testnet/tx/1e6db7cf0931c50e1a4a184551e455ff1337556b11affc8fdedc4b0550016346) |
| Admit A, B and C in one call | [`937c51af…e3aa`](https://stellar.expert/explorer/testnet/tx/937c51afca4502c5371e1f6d6b7525d9977d5988089b2a5411d8777a6d4be3aa) |
| Refuse obligations below 1 XLM | [`f93267fe…1405`](https://stellar.expert/explorer/testnet/tx/f93267fed346caee615e3ccdbb1af42487f3293544063132e8cc531f92b91405) |
| A deposits 10 XLM collateral | [`79bc9265…786c`](https://stellar.expert/explorer/testnet/tx/79bc9265c86cef9624dde2c1fec0a9f7b60aa2b48c3ecb86f686789fda0a786c) |
| A owes B 10 XLM | [`176a5ea1…8095`](https://stellar.expert/explorer/testnet/tx/176a5ea1e4275e7e668c836e273c34ad4dbc14a254f726784f19c3bef0928095) |
| B owes C 10 XLM with **no collateral**: B is already owed 10 this window | [`bfd62de6…afe3`](https://stellar.expert/explorer/testnet/tx/bfd62de64a50778bd683514282369de047d46ca4d87e1312688dd92559ebafe3) |
| C owes A 10 XLM | [`87a6aa06…65e3`](https://stellar.expert/explorer/testnet/tx/87a6aa0698878aa1f0d3cd1bd4a336205799ef216e78bf4a53f864ea41d665e3) |
| Settle: 30 XLM gross, every position 0, nothing moves | [`b968548c…944f`](https://stellar.expert/explorer/testnet/tx/b968548cd789dcf5a8559cede9a3356f68f7845d7ac39336a7743586b953944f) |
| A takes its collateral back with `withdraw_all` | [`86b6049e…62da`](https://stellar.expert/explorer/testnet/tx/86b6049eda46c82a4d7b6d6ef309a3f43c4f605de66bde241d28f70e401062da) |

## Interface

| Function | Who | What |
|---|---|---|
| `__constructor(admin)` | deployer | Sets the operator; windows may stay open 7 days before anyone can settle them |
| `admit(member)`, `admit_many(members)` | operator | Admits members (up to 32 at once, all or none) |
| `deposit(member, token, amount)` | member | Moves collateral into the contract |
| `withdraw(member, token, amount)`, `withdraw_all(member, token)` | member | Returns collateral not committed to the open window |
| `submit(obligations)` | every debtor in the batch | Records up to 32 obligations atomically; rejects the batch if any debtor's net debit is uncovered |
| `settle()` | operator, or anyone once the window is overdue | Applies every net position to balances and opens the next window |
| `set_token`, `set_min_amount`, `set_max_window`, `set_position_quota`, `set_suspended`, `pause`, `unpause` | operator | Operational controls; none can trap funds |
| `propose_admin`, `accept_admin` | operator, then successor | Two-step handover |
| `extend_ttl(member, token)` | anyone | Keeps a quiet member's entries from expiring |
| `balance`, `position`, `available`, `open_positions`, `gross`, `window`, `window_timing`, `is_member`, `is_suspended`, `token_allowed`, `min_amount`, `position_quota`, `reference_used`, `paused`, `admin` | anyone | Views |

Every state change publishes an event (`Admitted`, `Deposited`, `Withdrawn`,
`Obligated`, `PositionSettled` per member and token, `Settled`, and one per
operator action), so members can reconcile from events alone. The contract's
metadata names this repository, and releases publish the WASM with its SHA-256
for comparison with `stellar contract fetch`. TypeScript bindings come from
`stellar contract bindings typescript`; CI checks they compile. Settlement receipts can be proven to third parties with
[Externalize](https://github.com/Externalize-Labs/externalize).

## Guarantees

- **Settlement cannot fail.** After every batch, each member's net debit is
  covered by its balance; `settle` only moves numbers between balances.
- **Funds are never trapped.** Pausing, suspending a member or disallowing a
  token stops new risk only: available balances can always be withdrawn, and
  any window left open past its bound (1 minute to 30 days, never unset) can be
  settled by anyone.
- **It fits in one transaction.** A window holds at most 64 (member, token)
  positions and a batch at most 32 obligations; one member's obligations may
  open at most 16 of those positions per window (`position_quota`), so no
  single member can lock others out. Measured against testnet's
  limits, the largest settle uses 132 of 200 ledger writes and a full batch 11
  of 16 KB of events; a test fails if either passes 75%.
- **Immutable.** There is no upgrade function: the operator cannot replace the
  code holding members' collateral.

## Build and test

```sh
cargo test                 # unit, randomised-invariant and resource-limit tests
stellar contract build     # target/wasm32v1-none/release/setoff_settlement.wasm, about 15 KB
NETWORK=testnet scripts/testnet-demo.sh
```

The invariant test drives random deposits, batches, withdrawals, settlements,
pauses, suspensions and minimums, and checks after every step that tokens held
equal balances, positions sum to zero, every net debit is covered, and rejected
calls change nothing.

## Related

- [setoff-engine](https://github.com/SetOff-Org/setoff-engine): the off-chain
  netting algorithm and its reference vectors.
- [setoff-clearing](https://github.com/SetOff-Org/setoff-clearing): the clearing
  service; `setoff soroban` turns a closed window into calls on this contract.

## Status

Not audited. See [SECURITY.md](SECURITY.md).

## License

[Apache-2.0](LICENSE)
