# Security policy

The settlement contract holds members' collateral. Report vulnerabilities
privately through
[GitHub security advisories](https://github.com/SetOff-Org/setoff-contracts/security/advisories/new),
not public issues. We aim to acknowledge reports within three days.

In scope, most severe first:

1. Withdrawing more than your available balance, or moving another member's funds.
2. Recording an obligation without the debtor's authorization.
3. Leaving a net debit uncovered so that `settle` fails or a balance goes negative.
4. Blocking settlement or withdrawals for other members.

Invariants the code is written to keep, and that reports should be measured
against:

- `balance + min(position, 0) >= 0` for every member and token, always.
- The sum of positions in a window, per token, is zero.
- Token balances held by the contract equal the sum of member balances.
- No operator action (pause, suspension, token or minimum changes) stops a
  member withdrawing its available balance, and no window can stay open past
  its bound, which cannot be unset.

`src/invariants.rs` checks these after every step of randomised operation
sequences.

Trust in the operator, by design:

- The operator decides who may join, which tokens are allowed, and may pause
  new deposits and obligations or suspend a member. It cannot move funds,
  record obligations, or upgrade the contract: there is no upgrade function.
- The operator can settle early, which only applies positions members already
  authorized.

Known limits:

- A member can fill a window's 64 positions with small obligations, blocking
  new obligations until the window settles. `set_min_amount` makes this costly
  and `set_suspended` stops the member; a structural fix is open work.
- Only vetted tokens should be allowed: the contract calls the token's
  `transfer` on deposit and withdrawal and trusts its result.

The contract has not been audited.
