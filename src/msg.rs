use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Binary, Uint128};

#[cw_serde]
pub struct InstantiateMsg {
    /// Defaults to the instantiator if omitted.
    pub admin: Option<String>,
    pub choice_router: String,
    pub hallswap_router: String,
}

#[cw_serde]
pub enum Venue {
    Choice,
    Hallswap,
}

/// Parameters shared by both entrypoints.
#[cw_serde]
pub struct SwapParams {
    pub venue: Venue,
    /// The venue-native route body, forwarded verbatim as the sub-router's execute msg
    /// (Choice: the `execute_route` object; HallSwap: the `execute_routes` object).
    pub route: Binary,
    /// Final asset the swap returns, used for the balance-diff payout accounting.
    pub ask_denom: String,
    /// Minimum acceptable output; the tx reverts if the realized output is less.
    pub minimum_receive: Uint128,
    /// Where proceeds go. Defaults to the caller (`origin` on the `injective_exec` path).
    pub recipient: Option<String>,
}

#[cw_serde]
pub enum ExecuteMsg {
    /// Public entrypoint. Offer coin supplied as funds.
    Swap(SwapParams),

    /// Envelope for Injective's `injective_exec` call interface:
    /// `{"injective_exec": {origin, name, args}}`. `args` decodes to `SwapParams`.
    InjectiveExec {
        origin: String,
        name: String,
        args: SwapParams,
    },

    /// Internal, self-only: runs after the sub-swap settles; asserts minimum_receive
    /// against the balance delta and pays the recipient.
    AssertAndPay {
        ask_denom: String,
        minimum_receive: Uint128,
        recipient: String,
        balance_before: Uint128,
    },

    /// Admin-only: update router addresses.
    UpdateConfig {
        choice_router: Option<String>,
        hallswap_router: Option<String>,
    },

    /// Admin-only: recover funds that landed in the contract but are not part of any in-flight
    /// swap (e.g. a router refunding unspent offer on a normal swap, or a stray transfer). The
    /// contract is non-custodial per swap, so this cannot touch in-flight funds — every swap
    /// settles atomically within its own transaction before any Withdraw could run.
    Withdraw {
        denom: String,
        amount: Uint128,
        to: Option<String>,
    },

    /// Admin-only: propose a new admin. Takes effect only when `new_admin` calls AcceptAdmin.
    ProposeAdmin { new_admin: String },

    /// Pending-admin-only: accept a pending admin transfer.
    AcceptAdmin {},
}

#[cw_serde]
pub struct MigrateMsg {}

#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    #[returns(crate::state::Config)]
    Config {},
}
