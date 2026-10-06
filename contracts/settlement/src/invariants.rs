//! Random operation sequences, checking the contract's invariants after every step.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::arithmetic_side_effects)]

extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, BytesN, Env, Vec};

use crate::{Obligation, Settlement, SettlementClient};

/// xorshift64*: small, seedable, good enough to drive a state machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

struct Model<'a> {
    env: Env,
    contract: SettlementClient<'a>,
    tokens: [Address; 2],
    members: std::vec::Vec<Address>,
    reference: u32,
    /// (succeeded, failed) per operation: deposit, submit, withdraw, settle.
    outcomes: [(u32, u32); 4],
}

impl Model<'_> {
    fn new(members: usize) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.cost_estimate().budget().reset_unlimited();
        let admin = Address::generate(&env);
        let id = env.register(Settlement, (&admin,));
        let contract = SettlementClient::new(&env, &id);
        let tokens = [(); 2].map(|_| env.register_stellar_asset_contract_v2(Address::generate(&env)).address());
        for t in &tokens {
            contract.set_token(t, &true);
        }
        let members = (0..members)
            .map(|_| {
                let m = Address::generate(&env);
                contract.admit(&m);
                for t in &tokens {
                    StellarAssetClient::new(&env, t).mint(&m, &1_000_000);
                }
                m
            })
            .collect();
        Model { env, contract, tokens, members, reference: 0, outcomes: [(0, 0); 4] }
    }

    /// Everything the invariants look at, for comparing before and after a failed call.
    fn state(&self) -> std::vec::Vec<(i128, i128)> {
        let mut out = std::vec::Vec::new();
        for m in &self.members {
            for t in &self.tokens {
                out.push((self.contract.balance(m, t), self.contract.position(m, t)));
            }
        }
        out
    }

    fn check(&self, step: usize) {
        for t in &self.tokens {
            let held = TokenClient::new(&self.env, t).balance(&self.contract.address);
            let (mut balances, mut nets) = (0i128, 0i128);
            for m in &self.members {
                let balance = self.contract.balance(m, t);
                let net = self.contract.position(m, t);
                assert!(balance >= 0, "step {step}: negative balance");
                assert!(balance + net.min(0) >= 0, "step {step}: an uncovered net debit");
                assert_eq!(self.contract.available(m, t), balance + net.min(0), "step {step}: available");
                balances += balance;
                nets += net;
            }
            assert_eq!(held, balances, "step {step}: tokens held differ from balances");
            assert_eq!(nets, 0, "step {step}: positions do not sum to zero");
        }
    }

    fn obligation(&mut self, rng: &mut Rng) -> Obligation {
        let n = self.members.len() as u64;
        let debtor = rng.below(n) as usize;
        let creditor = (debtor + 1 + rng.below(n - 1) as usize) % self.members.len();
        self.reference += 1;
        let mut r = [0u8; 32];
        r[..4].copy_from_slice(&self.reference.to_be_bytes());
        Obligation {
            debtor: self.members[debtor].clone(),
            creditor: self.members[creditor].clone(),
            token: self.tokens[rng.below(2) as usize].clone(),
            amount: 1 + rng.below(400) as i128,
            reference: BytesN::from_array(&self.env, &r),
        }
    }

    /// Now and then the operator pauses, suspends or sets a minimum. None of
    /// it may break an invariant or stop settlement.
    fn operate(&self, rng: &mut Rng) {
        match rng.below(40) {
            0 if self.contract.paused() => self.contract.unpause(),
            0 => self.contract.pause(),
            1 => {
                let m = &self.members[rng.below(self.members.len() as u64) as usize];
                self.contract.set_suspended(m, &!self.contract.is_suspended(m));
            }
            2 => self.contract.set_min_amount(&self.tokens[rng.below(2) as usize], &(rng.below(40) as i128)),
            _ => {}
        }
    }

    fn step(&mut self, rng: &mut Rng, step: usize) {
        self.operate(rng);
        let m = self.members[rng.below(self.members.len() as u64) as usize].clone();
        let t = self.tokens[rng.below(2) as usize].clone();
        let before = self.state();
        let op = rng.below(10);
        let failed = match op {
            0..=2 => self.contract.try_deposit(&m, &t, &(1 + rng.below(300) as i128)).is_err(),
            3..=6 => {
                let mut batch = Vec::new(&self.env);
                for _ in 0..1 + rng.below(6) {
                    batch.push_back(self.obligation(rng));
                }
                self.contract.try_submit(&batch).is_err()
            }
            7..=8 => self.contract.try_withdraw(&m, &t, &(1 + rng.below(300) as i128)).is_err(),
            _ => {
                let totals: std::vec::Vec<i128> = self
                    .tokens
                    .iter()
                    .map(|t| self.members.iter().map(|m| self.contract.balance(m, t)).sum())
                    .collect();
                self.contract.settle(); // must never fail
                for (t, total) in self.tokens.iter().zip(totals) {
                    let after: i128 = self.members.iter().map(|m| self.contract.balance(m, t)).sum();
                    assert_eq!(after, total, "step {step}: settlement changed the total");
                    for m in &self.members {
                        assert_eq!(self.contract.position(m, t), 0, "step {step}: position left after settle");
                    }
                }
                false
            }
        };
        let kind = match op {
            0..=2 => 0,
            3..=6 => 1,
            7..=8 => 2,
            _ => 3,
        };
        if failed {
            self.outcomes[kind].1 += 1;
            assert_eq!(self.state(), before, "step {step}: a rejected call changed state");
        } else {
            self.outcomes[kind].0 += 1;
        }
        self.check(step);
    }
}

#[test]
fn invariants_hold_under_random_operations() {
    let mut outcomes = [(0, 0); 4];
    for seed in [1, 0x5E7_0FF, 0xDEAD_BEEF] {
        let mut rng = Rng(seed);
        let mut model = Model::new(4);
        for step in 0..80 {
            model.step(&mut rng, step);
        }
        for (total, o) in outcomes.iter_mut().zip(model.outcomes) {
            total.0 += o.0;
            total.1 += o.1;
        }
    }
    // The run must exercise both paths, or the invariants hold vacuously.
    let [deposit, submit, withdraw, settle] = outcomes;
    assert!(
        deposit.0 > 0 && submit.0 > 0 && submit.1 > 0 && withdraw.0 > 0 && withdraw.1 > 0 && settle.0 > 0,
        "{outcomes:?}"
    );
}
