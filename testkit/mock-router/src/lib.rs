//! Test-fixture DEX router. On `mock_swap` it sends `return_amount` of `return_denom`
//! back to `info.sender` (i.e. ninjucks), simulating a swap output. Pre-fund it with the
//! ask asset. This exists ONLY to validate ninjucks' parse/forward/settle wrapper on a live
//! chain without depending on the real Choice/HallSwap venues. Not part of the aggregator.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    entry_point, to_json_binary, BankMsg, Binary, Coin, Deps, DepsMut, Empty, Env, MessageInfo,
    Response, StdResult, Uint128,
};

#[cw_serde]
pub struct InstantiateMsg {}

#[cw_serde]
pub enum ExecuteMsg {
    /// Mirrors an aggregator route body: pay the caller `return_amount` of `return_denom`.
    MockSwap {
        return_denom: String,
        return_amount: Uint128,
    },
}

#[entry_point]
pub fn instantiate(_d: DepsMut, _e: Env, _i: MessageInfo, _m: InstantiateMsg) -> StdResult<Response> {
    Ok(Response::new())
}

#[entry_point]
pub fn execute(_d: DepsMut, _e: Env, info: MessageInfo, m: ExecuteMsg) -> StdResult<Response> {
    match m {
        ExecuteMsg::MockSwap {
            return_denom,
            return_amount,
        } => Ok(Response::new()
            .add_message(BankMsg::Send {
                to_address: info.sender.to_string(),
                amount: vec![Coin {
                    denom: return_denom,
                    amount: return_amount,
                }],
            })
            .add_attribute("action", "mock_swap")),
    }
}

#[entry_point]
pub fn query(_d: Deps, _e: Env, _m: Empty) -> StdResult<Binary> {
    to_json_binary(&Empty {})
}
