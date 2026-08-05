use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;
use cw_storage_plus::Item;

#[cw_serde]
pub struct Config {
    /// Admin allowed to update routers, withdraw stuck funds, and initiate an admin transfer.
    /// Transfer is two-step (propose + accept) to prevent an accidental one-shot lock-out.
    pub admin: Addr,
    /// Proposed next admin; becomes `admin` only after it calls AcceptAdmin. None when no
    /// transfer is pending.
    pub pending_admin: Option<Addr>,
    /// Choice DEX Aggregator router (execute_route schema).
    pub choice_router: Addr,
    /// HallSwap router (execute_routes schema, supports `to` recipient).
    pub hallswap_router: Addr,
}

pub const CONFIG: Item<Config> = Item::new("config");
