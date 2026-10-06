//! SetOff settlement contract.
//!
//! Members deposit collateral, record what they owe each other during a
//! window, and at settlement only each member's **net** position moves,
//! as an internal balance update with no token transfers at all.
//!
//! The rule that makes this safe: after every batch of obligations, each
//! member's net debit in the open window must be covered by its balance.
//! Members can owe each other far more than they hold, as long as it nets out,
//! and settlement can never fail for lack of funds.

#![no_std]

use soroban_sdk::{
    Address, BytesN, Env, Vec, contract, contracterror, contractevent, contractimpl, contracttype, panic_with_error,
    token,
};

/// Most (member, token) positions one window may touch, keeping `settle`
/// within Soroban's per-transaction resource limits.
pub const MAX_POSITIONS: u32 = 64;
/// Most obligations in one `submit` call.
pub const MAX_BATCH: u32 = 32;

/// Default bound on how long a window stays open before anyone may settle it.
pub const DEFAULT_MAX_WINDOW: u64 = 7 * 24 * 3_600;
/// Shortest and longest bound the operator may set, in seconds. There is no
/// way to disable it: committed collateral must never wait on the operator.
pub const MAX_WINDOW_RANGE: (u64, u64) = (60, 30 * 24 * 3_600);

const DAY: u32 = 17_280;
const TTL_THRESHOLD: u32 = 7 * DAY;
const TTL_EXTEND: u32 = 30 * DAY;

/// Contract errors.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The address is not an admitted member.
    NotMember = 1,
    /// Amounts must be positive.
    InvalidAmount = 2,
    /// A member cannot owe itself.
    SelfObligation = 3,
    /// The debtor already used this reference.
    DuplicateReference = 4,
    /// The batch would leave a debtor's net debit uncovered by its balance.
    InsufficientCollateral = 5,
    /// The withdrawal exceeds the balance not committed to the open window.
    InsufficientAvailable = 6,
    /// The open window already touches `MAX_POSITIONS` positions.
    WindowFull = 7,
    /// The address is already a member.
    AlreadyMember = 8,
    /// Batches must hold between 1 and `MAX_BATCH` obligations.
    BadBatch = 9,
    /// An amount overflowed.
    Overflow = 10,
    /// The contract is paused: no deposits or new obligations.
    Paused = 11,
    /// No operator handover is pending.
    NoPendingAdmin = 12,
    /// The token has not been allowed by the operator.
    TokenNotAllowed = 13,
    /// The window bound is outside `MAX_WINDOW_RANGE`.
    BadWindow = 14,
    /// The member is suspended: no deposits or new obligations.
    Suspended = 15,
}

/// One obligation in a `submit` batch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Obligation {
    /// Who owes. Must authorize the call.
    pub debtor: Address,
    /// Who is owed.
    pub creditor: Address,
    /// SEP-41 token the obligation is denominated in.
    pub token: Address,
    /// Amount, in the token's units.
    pub amount: i128,
    /// Debtor-chosen idempotency key, e.g. a hash of the off-chain payment order.
    pub reference: BytesN<32>,
}

/// A member's net position in one token, as `open_positions` reports it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Position {
    /// The member.
    pub member: Address,
    /// The token.
    pub token: Address,
    /// Net: positive receives at settlement, negative pays.
    pub net: i128,
}

#[contracttype]
enum Key {
    Admin,
    PendingAdmin,
    Window,
    WindowOpened,
    MaxWindow,
    Paused,
    Member(Address),
    Balance(Address, Address),
    Net(u64, Address, Address),
    Positions(u64),
    Gross(u64, Address),
    Reference(Address, BytesN<32>),
    Token(Address),
    Suspended(Address),
}

/// A member was admitted.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Admitted {
    /// The new member.
    #[topic]
    pub member: Address,
}

/// Collateral was deposited.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deposited {
    /// Member.
    #[topic]
    pub member: Address,
    /// Token.
    #[topic]
    pub token: Address,
    /// Amount.
    pub amount: i128,
}

/// Balance was withdrawn.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Withdrawn {
    /// Member.
    #[topic]
    pub member: Address,
    /// Token.
    #[topic]
    pub token: Address,
    /// Amount.
    pub amount: i128,
}

/// An obligation entered the open window.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Obligated {
    /// Debtor.
    #[topic]
    pub debtor: Address,
    /// Creditor.
    #[topic]
    pub creditor: Address,
    /// Token.
    pub token: Address,
    /// Amount.
    pub amount: i128,
    /// Debtor's reference.
    pub reference: BytesN<32>,
    /// Window it belongs to.
    pub window: u64,
}

/// The operator allowed or disallowed a token.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenAllowed {
    /// Token contract.
    #[topic]
    pub token: Address,
    /// Whether new deposits and obligations may use it.
    pub allowed: bool,
}

/// The operator role changed hands.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChanged {
    /// Previous operator.
    #[topic]
    pub previous: Address,
    /// New operator.
    #[topic]
    pub admin: Address,
}

/// The operator proposed a successor, who must accept to take over.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminProposed {
    /// Current operator.
    #[topic]
    pub admin: Address,
    /// Proposed operator.
    #[topic]
    pub proposed: Address,
}

/// The operator suspended or reinstated a member.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuspensionChanged {
    /// The member.
    #[topic]
    pub member: Address,
    /// Whether the member is now suspended.
    pub suspended: bool,
}

/// The operator changed how long a window may stay open.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaxWindowChanged {
    /// New bound, in seconds.
    pub seconds: u64,
}

/// The operator paused or resumed the contract.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseChanged {
    /// Whether the contract is now paused.
    pub paused: bool,
}

/// A window was settled.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settled {
    /// The settled window.
    #[topic]
    pub window: u64,
    /// Non-zero (member, token) positions applied.
    pub positions: u32,
}

/// The settlement contract.
#[contract]
pub struct Settlement;

fn bump<K: soroban_sdk::IntoVal<Env, soroban_sdk::Val>>(env: &Env, key: &K) {
    env.storage().persistent().extend_ttl(key, TTL_THRESHOLD, TTL_EXTEND);
}

fn window(env: &Env) -> u64 {
    env.storage().instance().get(&Key::Window).unwrap_or(0)
}

fn get_i128(env: &Env, key: &Key) -> i128 {
    env.storage().persistent().get(key).unwrap_or(0)
}

fn put_i128(env: &Env, key: &Key, v: i128) {
    env.storage().persistent().set(key, &v);
    bump(env, key);
}

fn add(env: &Env, a: i128, b: i128) -> i128 {
    a.checked_add(b).unwrap_or_else(|| panic_with_error!(env, Error::Overflow))
}

fn require_member(env: &Env, who: &Address) {
    if !env.storage().persistent().has(&Key::Member(who.clone())) {
        panic_with_error!(env, Error::NotMember);
    }
}

fn admit_one(env: &Env, member: Address) {
    let key = Key::Member(member.clone());
    if env.storage().persistent().has(&key) {
        panic_with_error!(env, Error::AlreadyMember);
    }
    env.storage().persistent().set(&key, &true);
    bump(env, &key);
    Admitted { member }.publish(env);
}

/// A member that may take on new risk: admitted and not suspended.
fn require_active(env: &Env, who: &Address) {
    require_member(env, who);
    if env.storage().persistent().get(&Key::Suspended(who.clone())).unwrap_or(false) {
        panic_with_error!(env, Error::Suspended);
    }
}

fn admin(env: &Env) -> Address {
    env.storage().instance().get(&Key::Admin).unwrap_or_else(|| panic_with_error!(env, Error::NotMember))
}

fn require_token(env: &Env, token: &Address) {
    if !env.storage().persistent().get(&Key::Token(token.clone())).unwrap_or(false) {
        panic_with_error!(env, Error::TokenNotAllowed);
    }
}

fn require_running(env: &Env) {
    if env.storage().instance().get(&Key::Paused).unwrap_or(false) {
        panic_with_error!(env, Error::Paused);
    }
}

fn touch(env: &Env) {
    env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_EXTEND);
}

/// Balance minus the member's net debit in the open window.
fn available_of(env: &Env, member: &Address, token: &Address) -> i128 {
    let balance = get_i128(env, &Key::Balance(member.clone(), token.clone()));
    let net = get_i128(env, &Key::Net(window(env), member.clone(), token.clone()));
    add(env, balance, net.min(0))
}

#[contractimpl]
impl Settlement {
    /// Sets the operator, who admits members and closes windows.
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&Key::Admin, &admin);
        env.storage().instance().set(&Key::Window, &0u64);
        env.storage().instance().set(&Key::WindowOpened, &env.ledger().timestamp());
        env.storage().instance().set(&Key::MaxWindow, &DEFAULT_MAX_WINDOW);
    }

    /// Admits a member. Operator only.
    pub fn admit(env: Env, member: Address) {
        admin(&env).require_auth();
        admit_one(&env, member);
        touch(&env);
    }

    /// Admits up to `MAX_BATCH` members at once, all or none. Operator only.
    pub fn admit_many(env: Env, members: Vec<Address>) {
        admin(&env).require_auth();
        if members.is_empty() || members.len() > MAX_BATCH {
            panic_with_error!(&env, Error::BadBatch);
        }
        for member in members.iter() {
            admit_one(&env, member);
        }
        touch(&env);
    }

    /// Moves `amount` of `token` from the member into the contract as collateral.
    pub fn deposit(env: Env, member: Address, token: Address, amount: i128) {
        member.require_auth();
        require_running(&env);
        require_active(&env, &member);
        require_token(&env, &token);
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        token::Client::new(&env, &token).transfer(&member, env.current_contract_address(), &amount);
        let key = Key::Balance(member.clone(), token.clone());
        let balance = add(&env, get_i128(&env, &key), amount);
        put_i128(&env, &key, balance);
        touch(&env);
        Deposited { member, token, amount }.publish(&env);
    }

    /// Returns `amount` of `token` to the member, if not committed to the open window.
    pub fn withdraw(env: Env, member: Address, token: Address, amount: i128) {
        member.require_auth();
        require_member(&env, &member);
        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if amount > available_of(&env, &member, &token) {
            panic_with_error!(&env, Error::InsufficientAvailable);
        }
        let key = Key::Balance(member.clone(), token.clone());
        let balance = get_i128(&env, &key);
        put_i128(&env, &key, balance.checked_sub(amount).unwrap_or_else(|| panic_with_error!(&env, Error::Overflow)));
        token::Client::new(&env, &token).transfer(&env.current_contract_address(), &member, &amount);
        touch(&env);
        Withdrawn { member, token, amount }.publish(&env);
    }

    /// Withdraws everything not committed to the open window and returns the
    /// amount (0 if nothing is available), so callers need not read
    /// `available` first and race a concurrent obligation.
    pub fn withdraw_all(env: Env, member: Address, token: Address) -> i128 {
        member.require_auth();
        require_member(&env, &member);
        let amount = available_of(&env, &member, &token);
        if amount <= 0 {
            return 0;
        }
        let key = Key::Balance(member.clone(), token.clone());
        let balance = get_i128(&env, &key);
        put_i128(&env, &key, balance.checked_sub(amount).unwrap_or_else(|| panic_with_error!(&env, Error::Overflow)));
        token::Client::new(&env, &token).transfer(&env.current_contract_address(), &member, &amount);
        touch(&env);
        Withdrawn { member, token, amount }.publish(&env);
        amount
    }

    /// Records a batch of obligations in the open window, atomically.
    ///
    /// Every debtor must authorize. After the whole batch is applied, each
    /// debtor's net debit must be covered by its balance; otherwise nothing
    /// is recorded. Returns the window the obligations joined.
    pub fn submit(env: Env, obligations: Vec<Obligation>) -> u64 {
        require_running(&env);
        if obligations.is_empty() || obligations.len() > MAX_BATCH {
            panic_with_error!(&env, Error::BadBatch);
        }
        let w = window(&env);
        let positions_key = Key::Positions(w);
        let mut positions: Vec<(Address, Address)> =
            env.storage().persistent().get(&positions_key).unwrap_or_else(|| Vec::new(&env));
        let mut debtors: Vec<(Address, Address)> = Vec::new(&env);
        let mut authorized: Vec<Address> = Vec::new(&env);

        for o in obligations.iter() {
            if !authorized.contains(&o.debtor) {
                o.debtor.require_auth();
                authorized.push_back(o.debtor.clone());
            }
            require_active(&env, &o.debtor);
            require_active(&env, &o.creditor);
            require_token(&env, &o.token);
            if o.amount <= 0 {
                panic_with_error!(&env, Error::InvalidAmount);
            }
            if o.debtor == o.creditor {
                panic_with_error!(&env, Error::SelfObligation);
            }
            let reference = Key::Reference(o.debtor.clone(), o.reference.clone());
            if env.storage().persistent().has(&reference) {
                panic_with_error!(&env, Error::DuplicateReference);
            }
            env.storage().persistent().set(&reference, &true);
            bump(&env, &reference);

            let debit = o.amount.checked_neg().unwrap_or_else(|| panic_with_error!(&env, Error::Overflow));
            for (member, delta) in [(&o.debtor, debit), (&o.creditor, o.amount)] {
                let pair = (member.clone(), o.token.clone());
                if !positions.contains(&pair) {
                    if positions.len() >= MAX_POSITIONS {
                        panic_with_error!(&env, Error::WindowFull);
                    }
                    positions.push_back(pair);
                }
                let key = Key::Net(w, member.clone(), o.token.clone());
                put_i128(&env, &key, add(&env, get_i128(&env, &key), delta));
            }
            let gross = Key::Gross(w, o.token.clone());
            put_i128(&env, &gross, add(&env, get_i128(&env, &gross), o.amount));

            let pair = (o.debtor.clone(), o.token.clone());
            if !debtors.contains(&pair) {
                debtors.push_back(pair);
            }
            Obligated {
                debtor: o.debtor,
                creditor: o.creditor,
                token: o.token,
                amount: o.amount,
                reference: o.reference,
                window: w,
            }
            .publish(&env);
        }

        for (debtor, token) in debtors.iter() {
            if available_of(&env, &debtor, &token) < 0 {
                panic_with_error!(&env, Error::InsufficientCollateral);
            }
        }
        env.storage().persistent().set(&positions_key, &positions);
        bump(&env, &positions_key);
        touch(&env);
        w
    }

    /// Closes the open window: applies every net position to balances and opens the next window.
    ///
    /// Cannot fail for lack of funds: `submit` already guaranteed every net debit is covered.
    /// The operator may settle at any time; once the window has been open for
    /// `max_window` seconds, anyone may, so a missing operator can never hold
    /// committed collateral hostage.
    pub fn settle(env: Env) -> u64 {
        let opened: u64 = env.storage().instance().get(&Key::WindowOpened).unwrap_or(0);
        let max: u64 = env.storage().instance().get(&Key::MaxWindow).unwrap_or(0);
        let overdue = max > 0 && env.ledger().timestamp() >= opened.saturating_add(max);
        if !overdue {
            admin(&env).require_auth();
        }
        let w = window(&env);
        let positions_key = Key::Positions(w);
        let positions: Vec<(Address, Address)> =
            env.storage().persistent().get(&positions_key).unwrap_or_else(|| Vec::new(&env));
        let mut applied: u32 = 0;
        for (member, token) in positions.iter() {
            let net_key = Key::Net(w, member.clone(), token.clone());
            let net = get_i128(&env, &net_key);
            env.storage().persistent().remove(&net_key);
            env.storage().persistent().remove(&Key::Gross(w, token.clone()));
            if net != 0 {
                let key = Key::Balance(member, token);
                put_i128(&env, &key, add(&env, get_i128(&env, &key), net));
                applied = applied.saturating_add(1);
            }
        }
        env.storage().persistent().remove(&positions_key);
        let next = w.checked_add(1).unwrap_or_else(|| panic_with_error!(&env, Error::Overflow));
        env.storage().instance().set(&Key::Window, &next);
        env.storage().instance().set(&Key::WindowOpened, &env.ledger().timestamp());
        touch(&env);
        Settled { window: w, positions: applied }.publish(&env);
        w
    }

    /// Allows or disallows a token for new deposits and obligations. Operator only.
    ///
    /// Only vetted tokens (for example Stellar Asset Contracts) should be
    /// allowed: the contract calls the token's `transfer` on deposit and
    /// withdrawal. Disallowing a token never blocks withdrawing it.
    pub fn set_token(env: Env, token: Address, allowed: bool) {
        admin(&env).require_auth();
        let key = Key::Token(token.clone());
        env.storage().persistent().set(&key, &allowed);
        bump(&env, &key);
        touch(&env);
        TokenAllowed { token, allowed }.publish(&env);
    }

    /// Whether a token may be used for new deposits and obligations.
    pub fn token_allowed(env: Env, token: Address) -> bool {
        env.storage().persistent().get(&Key::Token(token)).unwrap_or(false)
    }

    /// Sets how long a window may stay open before anyone may settle it,
    /// within `MAX_WINDOW_RANGE`. Operator only.
    pub fn set_max_window(env: Env, seconds: u64) {
        admin(&env).require_auth();
        if seconds < MAX_WINDOW_RANGE.0 || seconds > MAX_WINDOW_RANGE.1 {
            panic_with_error!(&env, Error::BadWindow);
        }
        env.storage().instance().set(&Key::MaxWindow, &seconds);
        touch(&env);
        MaxWindowChanged { seconds }.publish(&env);
    }

    /// When the open window opened (unix seconds) and its maximum duration.
    pub fn window_timing(env: Env) -> (u64, u64) {
        (
            env.storage().instance().get(&Key::WindowOpened).unwrap_or(0),
            env.storage().instance().get(&Key::MaxWindow).unwrap_or(0),
        )
    }

    /// Stops deposits and new obligations. Withdrawals of available funds and
    /// settlement keep working, so members can always get their money out.
    /// Operator only.
    pub fn pause(env: Env) {
        admin(&env).require_auth();
        env.storage().instance().set(&Key::Paused, &true);
        touch(&env);
        PauseChanged { paused: true }.publish(&env);
    }

    /// Resumes normal operation. Operator only.
    pub fn unpause(env: Env) {
        admin(&env).require_auth();
        env.storage().instance().set(&Key::Paused, &false);
        touch(&env);
        PauseChanged { paused: false }.publish(&env);
    }

    /// Suspends or reinstates a member. A suspended member cannot deposit or
    /// take part in new obligations, but its positions in the open window
    /// still settle and its available balance can always be withdrawn.
    /// Operator only.
    pub fn set_suspended(env: Env, member: Address, suspended: bool) {
        admin(&env).require_auth();
        require_member(&env, &member);
        let key = Key::Suspended(member.clone());
        env.storage().persistent().set(&key, &suspended);
        bump(&env, &key);
        touch(&env);
        SuspensionChanged { member, suspended }.publish(&env);
    }

    /// Whether a member is suspended.
    pub fn is_suspended(env: Env, member: Address) -> bool {
        env.storage().persistent().get(&Key::Suspended(member)).unwrap_or(false)
    }

    /// Extends the lifetime of a member's membership and balance entries,
    /// and the contract's own. Anyone may call it, e.g. a keeper for members
    /// that hold funds without trading, whose entries would otherwise expire
    /// and need restoring.
    pub fn extend_ttl(env: Env, member: Address, token: Address) {
        let storage = env.storage().persistent();
        for key in [Key::Member(member.clone()), Key::Balance(member, token)] {
            if storage.has(&key) {
                bump(&env, &key);
            }
        }
        touch(&env);
    }

    /// Whether the contract is paused.
    pub fn paused(env: Env) -> bool {
        env.storage().instance().get(&Key::Paused).unwrap_or(false)
    }

    /// Proposes a new operator. Takes effect only when the new operator calls
    /// `accept_admin`, so a mistyped address can never lock the contract.
    pub fn propose_admin(env: Env, new_admin: Address) {
        let current = admin(&env);
        current.require_auth();
        env.storage().instance().set(&Key::PendingAdmin, &new_admin);
        touch(&env);
        AdminProposed { admin: current, proposed: new_admin }.publish(&env);
    }

    /// Completes a handover proposed with `propose_admin`. The proposed operator only.
    pub fn accept_admin(env: Env) {
        let pending: Address = env
            .storage()
            .instance()
            .get(&Key::PendingAdmin)
            .unwrap_or_else(|| panic_with_error!(&env, Error::NoPendingAdmin));
        pending.require_auth();
        let previous = admin(&env);
        env.storage().instance().set(&Key::Admin, &pending);
        env.storage().instance().remove(&Key::PendingAdmin);
        touch(&env);
        AdminChanged { previous, admin: pending }.publish(&env);
    }

    /// The operator.
    pub fn admin(env: Env) -> Address {
        admin(&env)
    }

    /// The open window's number.
    pub fn window(env: Env) -> u64 {
        window(&env)
    }

    /// Whether `who` is a member.
    pub fn is_member(env: Env, who: Address) -> bool {
        env.storage().persistent().has(&Key::Member(who))
    }

    /// Settled balance of a member.
    pub fn balance(env: Env, member: Address, token: Address) -> i128 {
        get_i128(&env, &Key::Balance(member, token))
    }

    /// Net position in the open window: positive receives, negative pays.
    pub fn position(env: Env, member: Address, token: Address) -> i128 {
        get_i128(&env, &Key::Net(window(&env), member, token))
    }

    /// Balance not committed to the open window: what can be withdrawn now.
    pub fn available(env: Env, member: Address, token: Address) -> i128 {
        available_of(&env, &member, &token)
    }

    /// Every non-zero position in the open window, in the order first touched.
    /// Bounded by `MAX_POSITIONS`.
    pub fn open_positions(env: Env) -> Vec<Position> {
        let w = window(&env);
        let pairs: Vec<(Address, Address)> =
            env.storage().persistent().get(&Key::Positions(w)).unwrap_or_else(|| Vec::new(&env));
        let mut out = Vec::new(&env);
        for (member, token) in pairs.iter() {
            let net = get_i128(&env, &Key::Net(w, member.clone(), token.clone()));
            if net != 0 {
                out.push_back(Position { member, token, net });
            }
        }
        out
    }

    /// Gross obligations in the open window for a token.
    pub fn gross(env: Env, token: Address) -> i128 {
        get_i128(&env, &Key::Gross(window(&env), token))
    }
}

#[cfg(test)]
mod invariants;
#[cfg(test)]
mod resources;
#[cfg(test)]
mod test;
