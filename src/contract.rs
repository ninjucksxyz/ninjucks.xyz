use cosmwasm_std::{
    entry_point, to_json_binary, BankMsg, Binary, Coin, CosmosMsg, Deps, DepsMut, Env,
    MessageInfo, Response, StdResult, SubMsg, Uint128, WasmMsg,
};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, MigrateMsg, QueryMsg, SwapParams, Venue};
use crate::state::{Config, CONFIG};

const CONTRACT_NAME: &str = "crates.io:ninjucks";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    let admin = match msg.admin {
        Some(a) => deps.api.addr_validate(&a)?,
        None => info.sender,
    };
    let cfg = Config {
        admin,
        pending_admin: None,
        choice_router: deps.api.addr_validate(&msg.choice_router)?,
        hallswap_router: deps.api.addr_validate(&msg.hallswap_router)?,
    };
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "instantiate")
        .add_attribute("contract", CONTRACT_NAME)
        .add_attribute("version", CONTRACT_VERSION))
}

#[entry_point]
pub fn migrate(deps: DepsMut, _env: Env, _msg: MigrateMsg) -> Result<Response, ContractError> {
    // In-place upgrade path (gated by the on-chain wasm admin set at instantiate). Refresh the
    // stored cw2 version; future versions add migration logic here keyed on the old version.
    cw2::set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    Ok(Response::new()
        .add_attribute("action", "migrate")
        .add_attribute("version", CONTRACT_VERSION))
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Swap(params) => {
            // Caller is info.sender; offer coin is info.funds.
            let caller = info.sender.to_string();
            do_swap(deps, env, &info.funds, caller, params)
        }
        ExecuteMsg::InjectiveExec { origin, name: _, args } => {
            // injective_exec envelope: funds arrive as info.funds; `origin` is the caller-of-record
            // used to default the recipient.
            do_swap(deps, env, &info.funds, origin, args)
        }
        ExecuteMsg::AssertAndPay {
            ask_denom,
            minimum_receive,
            recipient,
            balance_before,
        } => assert_and_pay(deps, env, info, ask_denom, minimum_receive, recipient, balance_before),
        ExecuteMsg::UpdateConfig {
            choice_router,
            hallswap_router,
        } => update_config(deps, info, choice_router, hallswap_router),
        ExecuteMsg::Withdraw { denom, amount, to } => withdraw(deps, info, denom, amount, to),
        ExecuteMsg::ProposeAdmin { new_admin } => propose_admin(deps, info, new_admin),
        ExecuteMsg::AcceptAdmin {} => accept_admin(deps, info),
    }
}

/// Core routing. Fires one venue sub-swap (output must land back on this contract), then an
/// ordered self-message that enforces minimum_receive against the balance delta and pays out.
fn do_swap(
    deps: DepsMut,
    env: Env,
    funds: &[Coin],
    caller: String,
    params: SwapParams,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;

    // Exactly one offer coin.
    if funds.len() != 1 {
        return Err(ContractError::OfferCoinRequired);
    }
    let offer = funds[0].clone();

    // minimum_receive == 0 would disable the slippage guard (a mis-routed or hostile route could
    // consume the offer and deliver nothing while the tx still "succeeds"). Require a real floor.
    if params.minimum_receive.is_zero() {
        return Err(ContractError::ZeroMinimumReceive);
    }

    // Validate the recipient. Defaulting to the caller also validates a caller-supplied `origin`
    // on the injective_exec path (which, unlike info.sender, is user-controllable).
    let recipient = match params.recipient {
        Some(r) => deps.api.addr_validate(&r)?.to_string(),
        None => deps.api.addr_validate(&caller)?.to_string(),
    };

    let router = match params.venue {
        Venue::Choice => cfg.choice_router,
        Venue::Hallswap => cfg.hallswap_router,
    };

    // Baseline ask balance for the delta accounting. The offer coin is already credited to the
    // contract when do_swap runs. When the offer and ask denom are the SAME (an X -> ... -> X
    // cycle), that offer is included in the current balance and is about to be spent into the swap —
    // so we must exclude it from the baseline, otherwise `balance_now - baseline` underflows. When
    // denoms differ, the offer leaving does not touch the ask balance and the baseline is just the
    // current balance.
    let ask_balance_now = deps
        .querier
        .query_balance(&env.contract.address, &params.ask_denom)?
        .amount;
    let balance_before = if offer.denom == params.ask_denom {
        ask_balance_now.checked_sub(offer.amount)?
    } else {
        ask_balance_now
    };

    // (1) Sub-swap: forward the venue-native route body verbatim, with the offer as funds.
    //     The route must direct output back to THIS contract (HallSwap: `to`=this contract;
    //     Choice: pays info.sender = this contract). If it does not, balance_before == balance_now,
    //     received == 0, and AssertAndPay reverts — so mis-routing is safe, never a loss.
    let swap_msg = SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: router.to_string(),
        msg: params.route,
        funds: vec![offer],
    }));

    // (2) Ordered self-call: assert minimum_receive on the delta and pay the recipient.
    let pay_msg = SubMsg::new(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: env.contract.address.to_string(),
        msg: to_json_binary(&ExecuteMsg::AssertAndPay {
            ask_denom: params.ask_denom.clone(),
            minimum_receive: params.minimum_receive,
            recipient: recipient.clone(),
            balance_before,
        })?,
        funds: vec![],
    }));

    Ok(Response::new()
        .add_submessage(swap_msg)
        .add_submessage(pay_msg)
        .add_attribute("action", "swap")
        .add_attribute("ask_denom", params.ask_denom)
        .add_attribute("recipient", recipient))
    // The response sets no `data`: a swap is a plain routing call and returns no action.
}

fn assert_and_pay(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    ask_denom: String,
    minimum_receive: Uint128,
    recipient: String,
    balance_before: Uint128,
) -> Result<Response, ContractError> {
    // Self-only.
    if info.sender != env.contract.address {
        return Err(ContractError::Unauthorized);
    }

    let balance_now = deps
        .querier
        .query_balance(&env.contract.address, &ask_denom)?
        .amount;
    let received = balance_now.checked_sub(balance_before)?;

    // Fail closed (and avoid emitting a zero-amount BankMsg::Send, which the bank module rejects).
    if received.is_zero() {
        return Err(ContractError::NothingReceived);
    }
    if received < minimum_receive {
        return Err(ContractError::SlippageExceeded {
            received,
            minimum: minimum_receive,
        });
    }

    let pay = BankMsg::Send {
        to_address: recipient.clone(),
        amount: vec![Coin {
            denom: ask_denom.clone(),
            amount: received,
        }],
    };

    Ok(Response::new()
        .add_message(pay)
        .add_attribute("action", "assert_and_pay")
        .add_attribute("received", received)
        .add_attribute("recipient", recipient))
}

fn update_config(
    deps: DepsMut,
    info: MessageInfo,
    choice_router: Option<String>,
    hallswap_router: Option<String>,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.admin {
        return Err(ContractError::Unauthorized);
    }
    if let Some(c) = choice_router {
        cfg.choice_router = deps.api.addr_validate(&c)?;
    }
    if let Some(h) = hallswap_router {
        cfg.hallswap_router = deps.api.addr_validate(&h)?;
    }
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new().add_attribute("action", "update_config"))
}

/// Admin-only recovery of non-in-flight funds (stray transfers, offer-denom refunds from a
/// normal swap). Safe: swaps settle atomically within their own tx, so nothing is in-flight here.
fn withdraw(
    deps: DepsMut,
    info: MessageInfo,
    denom: String,
    amount: Uint128,
    to: Option<String>,
) -> Result<Response, ContractError> {
    let cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.admin {
        return Err(ContractError::Unauthorized);
    }
    let to = match to {
        Some(t) => deps.api.addr_validate(&t)?.to_string(),
        None => cfg.admin.to_string(),
    };
    Ok(Response::new()
        .add_message(BankMsg::Send {
            to_address: to.clone(),
            amount: vec![Coin { denom, amount }],
        })
        .add_attribute("action", "withdraw")
        .add_attribute("to", to))
}

/// Admin-only: propose a new admin. Two-step (see accept_admin) so a mistyped-but-valid address
/// cannot instantly lock config/withdraw forever.
fn propose_admin(
    deps: DepsMut,
    info: MessageInfo,
    new_admin: String,
) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    if info.sender != cfg.admin {
        return Err(ContractError::Unauthorized);
    }
    cfg.pending_admin = Some(deps.api.addr_validate(&new_admin)?);
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "propose_admin")
        .add_attribute("pending_admin", new_admin))
}

/// Pending-admin-only: complete the admin transfer.
fn accept_admin(deps: DepsMut, info: MessageInfo) -> Result<Response, ContractError> {
    let mut cfg = CONFIG.load(deps.storage)?;
    match &cfg.pending_admin {
        Some(p) if p == &info.sender => {}
        Some(_) => return Err(ContractError::Unauthorized),
        None => return Err(ContractError::NoPendingAdmin),
    }
    cfg.admin = info.sender;
    cfg.pending_admin = None;
    CONFIG.save(deps.storage, &cfg)?;
    Ok(Response::new()
        .add_attribute("action", "accept_admin")
        .add_attribute("admin", cfg.admin.to_string()))
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::Config {} => to_json_binary(&CONFIG.load(deps.storage)?),
    }
}
