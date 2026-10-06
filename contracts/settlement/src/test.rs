#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::arithmetic_side_effects)]

extern crate std;

use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, AuthorizedFunction, Events as _, Ledger as _};
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
        contract.set_token(&usdc, &true);
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

#[test]
fn only_allowed_tokens_can_be_deposited_or_owed() {
    let w = World::new(2);
    let other = w.env.register_stellar_asset_contract_v2(Address::generate(&w.env)).address();
    StellarAssetClient::new(&w.env, &other).mint(w.m(0), &100);
    assert!(!w.contract.token_allowed(&other));
    assert_eq!(w.contract.try_deposit(w.m(0), &other, &10).unwrap_err().unwrap(), code(Error::TokenNotAllowed));
    let mut ob = w.ob(0, 1, 1, 1);
    ob.token = other.clone();
    assert_eq!(w.contract.try_submit(&w.batch(&[ob])).unwrap_err().unwrap(), code(Error::TokenNotAllowed));

    // Disallowing a token never traps it.
    w.deposit(0, 50);
    w.contract.set_token(&w.usdc, &false);
    w.contract.withdraw(w.m(0), &w.usdc, &50);
}

#[test]
fn anyone_may_settle_an_overdue_window() {
    let w = World::new(2);
    w.env.ledger().with_mut(|l| l.timestamp = 1_000);
    w.contract.set_max_window(&3_600);
    w.contract.settle(); // operator settles window 0; window 1 opens at t=1000
    assert_eq!(w.contract.window_timing(), (1_000, 3_600));

    w.deposit(0, 10);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 10, 1)]));
    w.env.ledger().with_mut(|l| l.timestamp = 4_600);
    w.contract.settle();
    assert!(w.env.auths().is_empty(), "an overdue window needs no operator signature");
    assert_eq!(w.contract.balance(w.m(1), &w.usdc), 10);
}

#[test]
fn early_settlement_still_needs_the_operator() {
    let w = World::new(1);
    w.env.ledger().with_mut(|l| l.timestamp = 1_000);
    w.contract.set_max_window(&3_600);
    w.contract.settle();
    w.env.ledger().with_mut(|l| l.timestamp = 2_000);
    w.contract.settle();
    assert_eq!(w.env.auths()[0].0, w.admin);
}

#[test]
fn committed_collateral_is_never_stuck_without_the_operator() {
    // The operator never configures anything and then disappears.
    let w = World::new(2);
    w.deposit(0, 100);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 100, 1)]));
    assert_eq!(w.contract.available(w.m(0), &w.usdc), 0);

    let (opened, max) = w.contract.window_timing();
    assert!(max > 0, "a fresh contract must already bound how long a window stays open");
    w.env.ledger().with_mut(|l| l.timestamp = opened + max);
    w.env.mock_auths(&[]); // nobody signs
    w.contract.settle();
    assert_eq!(w.contract.balance(w.m(1), &w.usdc), 100);
}

#[test]
fn the_window_bound_cannot_be_disabled() {
    let w = World::new(1);
    for seconds in [0u64, 59, 31 * 24 * 3_600] {
        assert_eq!(w.contract.try_set_max_window(&seconds), Err(Ok(code(Error::BadWindow))), "{seconds}");
    }
    w.contract.set_max_window(&3_600);
    assert_eq!(w.contract.window_timing().1, 3_600);
}

/// Name (first topic) and topic/data values of the last event published.
fn last_event(env: &Env) -> (std::string::String, std::vec::Vec<soroban_sdk::xdr::ScVal>, soroban_sdk::xdr::ScVal) {
    use soroban_sdk::xdr::{ContractEventBody, ScVal};
    let events = env.events().all();
    let e = events.events().last().expect("no events").clone();
    let ContractEventBody::V0(body) = e.body;
    let mut topics: std::vec::Vec<ScVal> = body.topics.to_vec();
    let ScVal::Symbol(name) = topics.remove(0) else { panic!("first topic is not a symbol") };
    (name.to_utf8_string_lossy(), topics, body.data)
}

#[test]
fn governance_changes_are_published() {
    use soroban_sdk::xdr::{ScMapEntry, ScVal};
    let w = World::new(1);
    w.contract.set_max_window(&7_200);
    let (name, _, data) = last_event(&w.env);
    assert_eq!(name, "max_window_changed");
    let ScVal::Map(Some(map)) = data else { panic!("data is not a map") };
    assert!(map.iter().any(|ScMapEntry { val, .. }| *val == ScVal::U64(7_200)), "{map:?}");

    let next = Address::generate(&w.env);
    w.contract.propose_admin(&next);
    let (name, topics, _) = last_event(&w.env);
    assert_eq!(name, "admin_proposed");
    assert_eq!(topics.len(), 2, "both the current and proposed operator are topics");
}

#[test]
fn a_suspended_member_takes_no_new_risk_but_keeps_its_money() {
    let w = World::new(3);
    w.deposit(0, 100);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 40, 1)]));
    w.contract.set_suspended(w.m(0), &true);
    assert!(w.contract.is_suspended(w.m(0)));

    let suspended = code(Error::Suspended);
    assert_eq!(w.contract.try_deposit(w.m(0), &w.usdc, &1).unwrap_err().unwrap(), suspended);
    assert_eq!(w.contract.try_submit(&w.batch(&[w.ob(0, 2, 1, 2)])).unwrap_err().unwrap(), suspended, "as debtor");
    assert_eq!(w.contract.try_submit(&w.batch(&[w.ob(1, 0, 1, 3)])).unwrap_err().unwrap(), suspended, "as creditor");

    // What it already owes still settles, and what is free can leave.
    w.contract.withdraw(w.m(0), &w.usdc, &60);
    w.contract.settle();
    assert_eq!(w.contract.balance(w.m(0), &w.usdc), 0);
    assert_eq!(w.contract.balance(w.m(1), &w.usdc), 40);

    w.contract.set_suspended(w.m(0), &false);
    w.deposit(0, 1);
    let stranger = Address::generate(&w.env);
    assert_eq!(w.contract.try_set_suspended(&stranger, &true), Err(Ok(code(Error::NotMember))));
}

#[test]
fn open_positions_lists_the_non_zero_nets() {
    let w = World::new(3);
    w.deposit(0, 100);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 50, 1), w.ob(1, 2, 50, 2)]));
    // Member 1 received and paid 50: net zero, so not listed.
    let positions = w.contract.open_positions();
    assert_eq!(positions.len(), 2);
    let net = |i: usize| positions.iter().find(|p| &p.member == w.m(i)).map(|p| p.net);
    assert_eq!((net(0), net(1), net(2)), (Some(-50), None, Some(50)));
    let total: i128 = positions.iter().map(|p| p.net).sum();
    assert_eq!(total, 0, "positions always sum to zero");

    w.contract.settle();
    assert!(w.contract.open_positions().is_empty());
}

#[test]
fn anyone_can_keep_a_quiet_members_entries_alive() {
    let w = World::new(1);
    w.deposit(0, 10);
    let balance = crate::Key::Balance(w.m(0).clone(), w.usdc.clone());
    let ttl = || w.env.as_contract(&w.contract.address, || w.env.storage().persistent().get_ttl(&balance));
    let start = ttl();
    // Some weeks pass with no activity: the entry ages.
    w.env.ledger().with_mut(|l| l.sequence_number += 25 * 17_280);
    assert!(ttl() < start - 20 * 17_280);
    w.env.mock_auths(&[]);
    w.contract.extend_ttl(w.m(0), &w.usdc);
    assert!(ttl() >= 30 * 17_280 - 1, "extended to the full horizon");
    // Unknown members and tokens are a no-op, not an error.
    w.contract.extend_ttl(&Address::generate(&w.env), &w.usdc);
}

#[test]
fn members_can_be_admitted_in_one_batch() {
    let w = World::new(0);
    let fresh: std::vec::Vec<Address> = (0..3).map(|_| Address::generate(&w.env)).collect();
    let mut batch = Vec::new(&w.env);
    for m in &fresh {
        batch.push_back(m.clone());
    }
    w.contract.admit_many(&batch);
    assert!(fresh.iter().all(|m| w.contract.is_member(m)));

    // All or none: one existing member rejects the whole batch.
    let newcomer = Address::generate(&w.env);
    let mixed = Vec::from_array(&w.env, [newcomer.clone(), fresh[0].clone()]);
    assert_eq!(w.contract.try_admit_many(&mixed).unwrap_err().unwrap(), code(Error::AlreadyMember));
    assert!(!w.contract.is_member(&newcomer));
    assert_eq!(w.contract.try_admit_many(&Vec::new(&w.env)).unwrap_err().unwrap(), code(Error::BadBatch));
}

#[test]
fn withdraw_all_takes_exactly_what_is_free() {
    let w = World::new(2);
    w.deposit(0, 100);
    w.contract.submit(&w.batch(&[w.ob(0, 1, 30, 1)]));
    assert_eq!(w.contract.withdraw_all(w.m(0), &w.usdc), 70);
    assert_eq!(w.contract.balance(w.m(0), &w.usdc), 30, "the committed 30 stays");
    assert_eq!(w.contract.withdraw_all(w.m(0), &w.usdc), 0, "nothing left to take");
    w.contract.settle();
    assert_eq!(w.contract.withdraw_all(w.m(1), &w.usdc), 30);
    assert_eq!(TokenClient::new(&w.env, &w.usdc).balance(&w.contract.address), 0);
}
