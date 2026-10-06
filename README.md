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

Contract [`CCW6QCOS…EYQV`](https://stellar.expert/explorer/testnet/contract/CCW6QCOSJTTJHDXJOVQ36NVIBAHBUVPMSZNOIJ3A444O6YMVHR4ZEYQV),
using native XLM as the settlement token. [`scripts/testnet-demo.sh`](scripts/testnet-demo.sh)
deploys a fresh copy and replays this:

| Step | Transaction |
|---|---|
| Allow native XLM | [`9060d30a…986b`](https://stellar.expert/explorer/testnet/tx/9060d30a4065041ab551df394abe43426218ca22d9e78755ddc20032498c986b) |
| Admit A, B and C in one call | [`72deba3c…6dc2`](https://stellar.expert/explorer/testnet/tx/72deba3cfbf09f9c52a5d43a9b2d35761f4d5719153c15dfae77491a91336dc2) |
| Refuse obligations below 1 XLM | [`5a16a3d1…d36e`](https://stellar.expert/explorer/testnet/tx/5a16a3d1e4f9536db4299f8145580c0093fb22953c0b02b4e1886bcd9214d36e) |
| A deposits 10 XLM collateral | [`f39fb170…8a53`](https://stellar.expert/explorer/testnet/tx/f39fb170bb5eeb0f852dadee931e8d180bc90e452af1682533d0d73041688a53) |
| A owes B 10 XLM | [`00bcc214…420e`](https://stellar.expert/explorer/testnet/tx/00bcc2141577ffc6b56caf97eaf25ac710f4028baf644eb5371a9d05a2f4420e) |
| B owes C 10 XLM with **no collateral**: B is already owed 10 this window | [`14746e26…e6cd`](https://stellar.expert/explorer/testnet/tx/14746e268121e63aeb45e8455749c3d3e8e46aaa3aea91c017685effbd58e6cd) |
| C owes A 10 XLM | [`555239b8…5297`](https://stellar.expert/explorer/testnet/tx/555239b878b3034683eaa8cba1e82393b53514b2f3f04371f31ce820f3a65297) |
| Settle: 30 XLM gross, every position 0, nothing moves | [`21925e89…b89c`](https://stellar.expert/explorer/testnet/tx/21925e89bb1b75861880fe836e0ee3e1048cb23439569b5ab3d326ce06f2b89c) |
| A takes its collateral back with `withdraw_all` | [`9d335065…dac9`](https://stellar.expert/explorer/testnet/tx/9d335065463faec6b4374ff28455a4e1b3fef99364eb32d578855a814a65dac9) |

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
stellar contract build     # target/wasm32v1-none/release/setoff_settlement.wasm, about 23 KB
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
