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

The contract has not been audited.
