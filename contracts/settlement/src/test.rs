#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::arithmetic_side_effects)]

extern crate std;

use soroban_sdk::testutils::{Address as _, AuthorizedFunction, Events as _};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, BytesN, Env, Vec};

use crate::{Error, MAX_BATCH, MAX_POSITIONS, Obligation, Settlement, SettlementClient};

/// The host-level error a contract error surfaces as through the client.
fn code(e: Error) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(e as u32)
}

struct World<'a> {
    env: Env,
    admin: Address,
    contract: SettlementClient<'a>,
    usdc: Address,
    members: std::vec::Vec<Address>,
}

impl World<'_> {
    fn new(members: usize) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let id = env.register(Settlement, (&admin,));
        let contract = SettlementClient::new(&env, &id);
        let usdc = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
        let members = (0..members)
            .map(|_| {
                let m = Address::generate(&env);
                contract.admit(&m);
                StellarAssetClient::new(&env, &usdc).mint(&m, &1_000_000);
                m
            })
            .collect();
        World { env, admin, contract, usdc, members }
    }

    fn m(&self, i: usize) -> &Address {
        &self.members[i]
    }

    fn ob(&self, debtor: usize, creditor: usize, amount: i128, tag: u8) -> Obligation {
        Obligation {
            debtor: self.m(debtor).clone(),
            creditor: self.m(creditor).clone(),
            token: self.usdc.clone(),
            amount,
            reference: BytesN::from_array(&self.env, &[tag; 32]),
        }
    }

    fn batch(&self, obs: &[Obligation]) -> Vec<Obligation> {
        let mut v = Vec::new(&self.env);
        for o in obs {
            v.push_back(o.clone());
        }
        v
    }

    fn deposit(&self, i: usize, amount: i128) {
        self.contract.deposit(self.m(i), &self.usdc, &amount);
    }
}

#[test]
fn a_perfect_cycle_settles_with_no_collateral_at_all() {
    let w = World::new(3);
    let batch = w.batch(&[w.ob(0, 1, 500, 1), w.ob(1, 2, 500, 2), w.ob(2, 0, 500, 3)]);
    assert_eq!(w.contract.submit(&batch), 0);
    assert_eq!(w.contract.gross(&w.usdc), 1_500);
    for i in 0..3 {
        assert_eq!(w.contract.position(w.m(i), &w.usdc), 0);
    }
    assert_eq!(w.contract.settle(), 0);
    assert_eq!(w.contract.window(), 1);
    for i in 0..3 {
        assert_eq!(w.contract.balance(w.m(i), &w.usdc), 0);
    }
}

#[test]
fn bilateral_obligations_only_need_the_difference_covered() {
    let w = World::new(2);
    w.deposit(0, 40);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 100, 1), w.ob(1, 0, 60, 2)]));
    assert_eq!(w.contract.position(w.m(0), &w.usdc), -40);
    assert_eq!(w.contract.available(w.m(0), &w.usdc), 0);
    w.contract.settle();
    assert_eq!(w.contract.balance(w.m(0), &w.usdc), 0);
    assert_eq!(w.contract.balance(w.m(1), &w.usdc), 40);

    // The creditor can now take its net receipt out as tokens.
    w.contract.withdraw(w.m(1), &w.usdc, &40);
    assert_eq!(TokenClient::new(&w.env, &w.usdc).balance(w.m(1)), 1_000_040);
}

#[test]
fn an_uncovered_net_debit_is_rejected() {
    let w = World::new(2);
    w.deposit(0, 50);
    let err = w.contract.try_submit(&w.batch(&[w.ob(0, 1, 100, 1)])).unwrap_err().unwrap();
    assert_eq!(err, code(Error::InsufficientCollateral));
    assert_eq!(w.contract.gross(&w.usdc), 0, "a rejected batch records nothing");
}

#[test]
fn offsetting_obligations_must_arrive_in_the_same_batch() {
    let w = World::new(2);
    w.deposit(0, 50);
    assert!(w.contract.try_submit(&w.batch(&[w.ob(0, 1, 100, 1)])).is_err());
    assert!(w.contract.try_submit(&w.batch(&[w.ob(0, 1, 100, 1), w.ob(1, 0, 60, 2)])).is_ok());
}

#[test]
fn collateral_committed_to_the_window_cannot_be_withdrawn() {
    let w = World::new(2);
    w.deposit(0, 100);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 80, 1)]));
    assert_eq!(w.contract.available(w.m(0), &w.usdc), 20);
    let err = w.contract.try_withdraw(w.m(0), &w.usdc, &30).unwrap_err().unwrap();
    assert_eq!(err, code(Error::InsufficientAvailable));
    w.contract.withdraw(w.m(0), &w.usdc, &20);
    w.contract.settle();
    assert_eq!(w.contract.balance(w.m(0), &w.usdc), 0);
    assert_eq!(w.contract.balance(w.m(1), &w.usdc), 80);
}

#[test]
fn obligations_require_the_debtors_signature() {
    let w = World::new(2);
    w.deposit(0, 100);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 10, 1)]));
    let auths = w.env.auths();
    assert_eq!(auths.len(), 1);
    assert_eq!(&auths[0].0, w.m(0));
    assert!(matches!(auths[0].1.function, AuthorizedFunction::Contract(_)));
}

#[test]
fn settlement_and_admission_are_operator_only() {
    let w = World::new(1);
    w.contract.settle();
    assert_eq!(w.env.auths()[0].0, w.admin);
    let newcomer = Address::generate(&w.env);
    w.contract.admit(&newcomer);
    assert_eq!(w.env.auths()[0].0, w.admin);
    assert_eq!(w.contract.try_admit(&newcomer).unwrap_err().unwrap(), code(Error::AlreadyMember));
}

#[test]
fn invalid_obligations_are_rejected() {
    let w = World::new(2);
    w.deposit(0, 1_000);
    let outsider = Address::generate(&w.env);
    let mut stranger = w.ob(0, 1, 10, 1);
    stranger.creditor = outsider;
    let cases = [
        (w.ob(0, 1, 0, 1), Error::InvalidAmount),
        (w.ob(0, 0, 10, 1), Error::SelfObligation),
        (stranger, Error::NotMember),
    ];
    for (o, want) in cases {
        assert_eq!(w.contract.try_submit(&w.batch(&[o])).unwrap_err().unwrap(), code(want));
    }
    w.contract.submit(&w.batch(&[w.ob(0, 1, 10, 9)]));
    assert_eq!(
        w.contract.try_submit(&w.batch(&[w.ob(0, 1, 10, 9)])).unwrap_err().unwrap(),
        code(Error::DuplicateReference)
    );
    assert_eq!(w.contract.try_submit(&Vec::new(&w.env)).unwrap_err().unwrap(), code(Error::BadBatch));
    let too_many: std::vec::Vec<Obligation> = (0..=MAX_BATCH as u8).map(|i| w.ob(0, 1, 1, 100 + i)).collect();
    assert_eq!(w.contract.try_submit(&w.batch(&too_many)).unwrap_err().unwrap(), code(Error::BadBatch));
}

#[test]
fn windows_are_bounded() {
    let w = World::new(MAX_POSITIONS as usize + 2);
    w.deposit(0, 1_000_000);
    let mut tag = 0u8;
    for creditor in 1..MAX_POSITIONS as usize {
        tag = tag.wrapping_add(1);
        w.contract.submit(&w.batch(&[w.ob(0, creditor, 1, tag)]));
    }
    let overflow = w.ob(0, MAX_POSITIONS as usize, 1, 255);
    assert_eq!(w.contract.try_submit(&w.batch(&[overflow])).unwrap_err().unwrap(), code(Error::WindowFull));
}

#[test]
fn each_window_starts_clean() {
    let w = World::new(2);
    w.deposit(0, 100);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 70, 1)]));
    w.contract.settle();
    assert_eq!(w.contract.position(w.m(0), &w.usdc), 0);
    assert_eq!(w.contract.gross(&w.usdc), 0);
    assert_eq!(w.contract.available(w.m(0), &w.usdc), 30);
    w.contract.submit(&w.batch(&[w.ob(1, 0, 70, 2)]));
    w.contract.settle();
    assert_eq!(w.contract.balance(w.m(0), &w.usdc), 100);
    assert_eq!(w.contract.balance(w.m(1), &w.usdc), 0);
}

#[test]
fn settlement_publishes_an_event() {
    let w = World::new(2);
    w.deposit(0, 10);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 10, 1)]));
    w.contract.settle();
    assert!(!w.env.events().all().events().is_empty());
}

#[test]
fn pausing_stops_new_risk_but_never_traps_funds() {
    let w = World::new(2);
    w.deposit(0, 100);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 30, 1)]));
    w.contract.pause();
    assert!(w.contract.paused());
    assert_eq!(w.contract.try_deposit(w.m(0), &w.usdc, &1).unwrap_err().unwrap(), code(Error::Paused));
    assert_eq!(w.contract.try_submit(&w.batch(&[w.ob(0, 1, 1, 2)])).unwrap_err().unwrap(), code(Error::Paused));
    // Available funds can still leave, and the open window can still settle.
    w.contract.withdraw(w.m(0), &w.usdc, &70);
    w.contract.settle();
    assert_eq!(w.contract.balance(w.m(1), &w.usdc), 30);
    w.contract.unpause();
    w.deposit(0, 1);
}

#[test]
fn the_operator_role_changes_hands_in_two_steps() {
    let w = World::new(1);
    let next = Address::generate(&w.env);
    assert_eq!(w.contract.try_accept_admin().unwrap_err().unwrap(), code(Error::NoPendingAdmin));
    w.contract.propose_admin(&next);
    assert_eq!(w.contract.admin(), w.admin, "nothing changes until the new operator accepts");
    w.contract.accept_admin();
    assert_eq!(w.env.auths()[0].0, next, "acceptance is signed by the new operator");
    assert_eq!(w.contract.admin(), next);
    assert_eq!(w.contract.try_accept_admin().unwrap_err().unwrap(), code(Error::NoPendingAdmin));
}
