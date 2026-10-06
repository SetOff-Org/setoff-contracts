//! The worst-case calls must fit in one transaction, or "settlement cannot
//! fail" would not hold on a real network.
//!
//! Limits are Stellar testnet's per-transaction Soroban settings as of
//! October 2026 (`stellar network settings`); calls must stay within 75% of
//! each so a later network change has room to land.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::arithmetic_side_effects)]

extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::StellarAssetClient;
use soroban_sdk::{Address, BytesN, Env, Vec};

use crate::{MAX_BATCH, MAX_POSITIONS, Obligation, Settlement, SettlementClient};

const TX_MAX_INSTRUCTIONS: i64 = 400_000_000;
const TX_MAX_DISK_READ_ENTRIES: u32 = 200;
const TX_MAX_WRITE_ENTRIES: u32 = 200;
const TX_MAX_WRITE_BYTES: u32 = 132_096;
const TX_MAX_EVENTS_BYTES: u32 = 16_384;

fn within(what: &str, used: u64, limit: u64) {
    std::println!("{what}: {used} of {limit}");
    assert!(used * 4 <= limit * 3, "{what}: {used} exceeds 75% of the {limit} limit");
}

fn check(env: &Env, call: &str) {
    let r = env.cost_estimate().resources();
    within(&std::format!("{call} instructions"), r.instructions as u64, TX_MAX_INSTRUCTIONS as u64);
    within(&std::format!("{call} disk reads"), r.disk_read_entries.into(), TX_MAX_DISK_READ_ENTRIES.into());
    within(&std::format!("{call} writes"), r.write_entries.into(), TX_MAX_WRITE_ENTRIES.into());
    within(&std::format!("{call} write bytes"), r.write_bytes.into(), TX_MAX_WRITE_BYTES.into());
    within(&std::format!("{call} event bytes"), r.contract_events_size_bytes.into(), TX_MAX_EVENTS_BYTES.into());
}

#[test]
fn the_largest_window_settles_in_one_transaction() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract = SettlementClient::new(&env, &env.register(Settlement, (&admin,)));
    let token = env.register_stellar_asset_contract_v2(Address::generate(&env)).address();
    contract.set_token(&token, &true);

    // One token, MAX_POSITIONS members in a ring with unequal amounts, so
    // every position is non-zero and settle rewrites every balance.
    let n = MAX_POSITIONS as usize;
    let members: std::vec::Vec<Address> = (0..n)
        .map(|_| {
            let m = Address::generate(&env);
            contract.admit(&m);
            StellarAssetClient::new(&env, &token).mint(&m, &1_000);
            contract.deposit(&m, &token, &1_000);
            m
        })
        .collect();
    let obligations: std::vec::Vec<Obligation> = (0..n)
        .map(|i| Obligation {
            debtor: members[i].clone(),
            creditor: members[(i + 1) % n].clone(),
            token: token.clone(),
            amount: (i + 1) as i128,
            reference: BytesN::from_array(&env, &[i as u8; 32]),
        })
        .collect();

    // A full batch touching as many new positions as it can.
    for (b, chunk) in obligations.chunks(MAX_BATCH as usize).enumerate() {
        let mut batch = Vec::new(&env);
        for o in chunk {
            batch.push_back(o.clone());
        }
        contract.submit(&batch);
        check(&env, &std::format!("submit #{b}"));
    }
    assert_eq!(contract.open_positions().len(), MAX_POSITIONS);

    contract.settle();
    check(&env, "settle");
    assert!(contract.open_positions().is_empty());
}
