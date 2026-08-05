//! Integration tests for ninjucks.
//!
//! Focus (per scope): the `injective_exec` envelope parses into our handler, the sub-message is
//! forwarded verbatim to the correct router with the offer funds, and the response/payout is read
//! back with the minimum_receive guard enforced. The real Choice/HallSwap routers are replaced by
//! a mock router that returns a configurable amount — we are testing ninjucks' parse/forward/settle
//! wrapper, not the venues' swap math.

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    from_json, to_json_binary, BankMsg, Binary, Coin, Deps, DepsMut, Env, MessageInfo,
    Response, StdResult, Uint128,
};
use cw_multi_test::{App, AppBuilder, ContractWrapper, Executor};

use ninjucks::contract::{execute, instantiate, query};
use ninjucks::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, SwapParams, Venue};
use ninjucks::state::Config;

// ---------------------------------------------------------------------------
// Mock router: on `mock_swap`, sends `return_amount` of `return_denom` back to info.sender
// (i.e. ninjucks). Pre-funded with the ask asset. This stands in for Choice/HallSwap.
// ---------------------------------------------------------------------------

#[cw_serde]
struct MockInit {}

#[cw_serde]
enum MockExec {
    MockSwap {
        return_denom: String,
        return_amount: Uint128,
    },
}

fn mock_instantiate(_d: DepsMut, _e: Env, _i: MessageInfo, _m: MockInit) -> StdResult<Response> {
    Ok(Response::new())
}

fn mock_execute(_d: DepsMut, _e: Env, info: MessageInfo, m: MockExec) -> StdResult<Response> {
    match m {
        MockExec::MockSwap {
            return_denom,
            return_amount,
        } => Ok(Response::new().add_message(BankMsg::Send {
            to_address: info.sender.to_string(), // pay the caller = ninjucks
            amount: vec![Coin {
                denom: return_denom,
                amount: return_amount,
            }],
        })),
    }
}

fn mock_query(_d: Deps, _e: Env, _m: cosmwasm_std::Empty) -> StdResult<Binary> {
    to_json_binary(&())
}

// ---------------------------------------------------------------------------

const OFFER: &str = "uoffer";
const ASK: &str = "inj";

fn ninjucks_code(app: &mut App) -> u64 {
    app.store_code(Box::new(ContractWrapper::new(execute, instantiate, query)))
}
fn mock_code(app: &mut App) -> u64 {
    app.store_code(Box::new(ContractWrapper::new(
        mock_execute,
        mock_instantiate,
        mock_query,
    )))
}

struct World {
    app: App,
    ninjucks: cosmwasm_std::Addr,
    choice: cosmwasm_std::Addr,
    hallswap: cosmwasm_std::Addr,
    user: cosmwasm_std::Addr,
}

fn setup(mock_return: Uint128, router_ask_funding: u128) -> World {
    let admin_seed = "admin";
    let mut app = AppBuilder::new().build(|_r, _api, _s| {});
    let user = app.api().addr_make("user");
    let admin = app.api().addr_make(admin_seed);

    // Fund the user with the offer coin.
    app.init_modules(|router, _, storage| {
        router
            .bank
            .init_balance(storage, &user, vec![Coin::new(1_000_000u128, OFFER)])
            .unwrap();
    });

    let ncode = ninjucks_code(&mut app);
    let mcode = mock_code(&mut app);

    // Two mock routers standing in for Choice + HallSwap, each pre-funded with the ask asset.
    let choice = app
        .instantiate_contract(mcode, admin.clone(), &MockInit {}, &[], "choice", None)
        .unwrap();
    let hallswap = app
        .instantiate_contract(mcode, admin.clone(), &MockInit {}, &[], "hallswap", None)
        .unwrap();
    for r in [&choice, &hallswap] {
        app.init_modules(|router, _, storage| {
            router
                .bank
                .init_balance(storage, r, vec![Coin::new(router_ask_funding, ASK)])
                .unwrap();
        });
    }

    let ninjucks = app
        .instantiate_contract(
            ncode,
            admin.clone(),
            &InstantiateMsg {
                admin: Some(admin.to_string()),
                choice_router: choice.to_string(),
                hallswap_router: hallswap.to_string(),
            },
            &[],
            "ninjucks",
            None,
        )
        .unwrap();

    let _ = mock_return; // used by callers building the route
    World {
        app,
        ninjucks,
        choice,
        hallswap,
        user,
    }
}

fn route_for(mock_return: Uint128) -> Binary {
    to_json_binary(&MockExec::MockSwap {
        return_denom: ASK.to_string(),
        return_amount: mock_return,
    })
    .unwrap()
}

// ===========================================================================
// 1. The injective_exec envelope must parse into our handler.
// ===========================================================================
#[test]
fn injective_exec_envelope_deserializes() {
    let route = route_for(Uint128::new(1234)).to_base64();
    let raw = format!(
        r#"{{"injective_exec":{{"origin":"inj1originaddr","name":"swap","args":{{"venue":"hallswap","route":"{route}","ask_denom":"inj","minimum_receive":"1000","recipient":null}}}}}}"#
    );
    let msg: ExecuteMsg = from_json(raw.as_bytes()).unwrap();
    match msg {
        ExecuteMsg::InjectiveExec { origin, name, args } => {
            assert_eq!(origin, "inj1originaddr");
            assert_eq!(name, "swap");
            assert_eq!(args.ask_denom, "inj");
            assert_eq!(args.minimum_receive, Uint128::new(1000));
            assert!(matches!(args.venue, Venue::Hallswap));
        }
        _ => panic!("expected InjectiveExec variant"),
    }
}

// ===========================================================================
// 2. Public Swap path: forwards to the chosen router, reads the delta, pays the recipient.
// ===========================================================================
#[test]
fn public_swap_pays_recipient() {
    let ret = Uint128::new(500_000);
    let mut w = setup(ret, 10_000_000);
    let recipient = w.app.api().addr_make("recipient");

    w.app
        .execute_contract(
            w.user.clone(),
            w.ninjucks.clone(),
            &ExecuteMsg::Swap(SwapParams {
                venue: Venue::Choice,
                route: route_for(ret),
                ask_denom: ASK.to_string(),
                minimum_receive: Uint128::new(400_000),
                recipient: Some(recipient.to_string()),
            }),
            &[Coin::new(100_000u128, OFFER)],
        )
        .unwrap();

    // recipient got exactly the router's delivered amount; ninjucks holds nothing.
    let bal = w.app.wrap().query_balance(&recipient, ASK).unwrap();
    assert_eq!(bal.amount, ret);
    let dust = w.app.wrap().query_balance(&w.ninjucks, ASK).unwrap();
    assert_eq!(dust.amount, Uint128::zero());
    // offer was forwarded to the router (not stuck in ninjucks)
    let stuck = w.app.wrap().query_balance(&w.ninjucks, OFFER).unwrap();
    assert_eq!(stuck.amount, Uint128::zero());
    let router_got = w.app.wrap().query_balance(&w.choice, OFFER).unwrap();
    assert_eq!(router_got.amount, Uint128::new(100_000));
}

// ===========================================================================
// 3. Slippage: realized < minimum_receive => whole tx reverts (nothing moves).
// ===========================================================================
#[test]
fn slippage_reverts_atomically() {
    let ret = Uint128::new(100);
    let mut w = setup(ret, 10_000_000);
    let recipient = w.app.api().addr_make("recipient");

    let err = w.app.execute_contract(
        w.user.clone(),
        w.ninjucks.clone(),
        &ExecuteMsg::Swap(SwapParams {
            venue: Venue::Choice,
            route: route_for(ret),
            ask_denom: ASK.to_string(),
            minimum_receive: Uint128::new(1_000_000), // unreachable
            recipient: Some(recipient.to_string()),
        }),
        &[Coin::new(100_000u128, OFFER)],
    );
    assert!(err.is_err(), "expected slippage revert");

    // Atomic: recipient got nothing, user keeps their offer.
    let bal = w.app.wrap().query_balance(&recipient, ASK).unwrap();
    assert_eq!(bal.amount, Uint128::zero());
    let user_offer = w.app.wrap().query_balance(&w.user, OFFER).unwrap();
    assert_eq!(user_offer.amount, Uint128::new(1_000_000));
}

// ===========================================================================
// 4. injective_exec path end-to-end: envelope -> forward -> settle to `origin` by default.
// ===========================================================================
#[test]
fn injective_exec_pays_origin_by_default() {
    let ret = Uint128::new(250_000);
    let mut w = setup(ret, 10_000_000);
    // On the injective_exec path funds arrive as info.funds alongside the envelope.
    // We model that by calling InjectiveExec with the offer as funds
    // and origin = user (defaulting recipient to origin).
    w.app
        .execute_contract(
            w.user.clone(),
            w.ninjucks.clone(),
            &ExecuteMsg::InjectiveExec {
                origin: w.user.to_string(),
                name: "swap".to_string(),
                args: SwapParams {
                    venue: Venue::Hallswap,
                    route: route_for(ret),
                    ask_denom: ASK.to_string(),
                    minimum_receive: Uint128::new(200_000),
                    recipient: None, // default -> origin
                },
            },
            &[Coin::new(100_000u128, OFFER)],
        )
        .unwrap();

    // origin (user) received the ask; forwarded via the HALLSWAP router specifically.
    let bal = w.app.wrap().query_balance(&w.user, ASK).unwrap();
    assert_eq!(bal.amount, ret);
    let hall_got = w.app.wrap().query_balance(&w.hallswap, OFFER).unwrap();
    assert_eq!(hall_got.amount, Uint128::new(100_000));
    let choice_got = w.app.wrap().query_balance(&w.choice, OFFER).unwrap();
    assert_eq!(choice_got.amount, Uint128::zero()); // wrong router untouched
}

// ===========================================================================
// 4b. injective_exec must return empty response data (a swap returns no action).
// ===========================================================================
#[test]
fn injective_exec_returns_no_action_data() {
    let ret = Uint128::new(300_000);
    let mut w = setup(ret, 10_000_000);
    let resp = w
        .app
        .execute_contract(
            w.user.clone(),
            w.ninjucks.clone(),
            &ExecuteMsg::InjectiveExec {
                origin: w.user.to_string(),
                name: "anything".to_string(),
                args: SwapParams {
                    venue: Venue::Choice,
                    route: route_for(ret),
                    ask_denom: ASK.to_string(),
                    minimum_receive: Uint128::new(1),
                    recipient: None,
                },
            },
            &[Coin::new(100_000u128, OFFER)],
        )
        .unwrap();
    // ninjucks never sets Response.data.
    assert!(
        resp.data.is_none(),
        "injective_exec must not return response data: {:?}",
        resp.data
    );
}

// 4c. The envelope's `name` is opaque to us (only `args` matters); arbitrary names must still route.
#[test]
fn injective_exec_ignores_name_field() {
    let ret = Uint128::new(300_000);
    let mut w = setup(ret, 10_000_000);
    for name in ["", "swap", "arbitrary_op_name", "ROUTE"] {
        let before = w.app.wrap().query_balance(&w.user, ASK).unwrap().amount;
        w.app
            .execute_contract(
                w.user.clone(),
                w.ninjucks.clone(),
                &ExecuteMsg::InjectiveExec {
                    origin: w.user.to_string(),
                    name: name.to_string(),
                    args: SwapParams {
                        venue: Venue::Hallswap,
                        route: route_for(ret),
                        ask_denom: ASK.to_string(),
                        minimum_receive: Uint128::new(1),
                        recipient: None,
                    },
                },
                &[Coin::new(50_000u128, OFFER)],
            )
            .unwrap();
        let after = w.app.wrap().query_balance(&w.user, ASK).unwrap().amount;
        assert_eq!(after - before, ret, "name={name:?} should still route");
    }
}

// ===========================================================================
// 4d. Cycle case: offer denom == ask denom (X -> ... -> X round-trip). The offer is already in
//     the contract balance and gets spent; payout must be the swap OUTPUT, not output-minus-offer.
// ===========================================================================
#[test]
fn cycle_offer_equals_ask_pays_full_output() {
    // offer = ASK = "inj". User sends 100_000; router returns 130_000 of the same denom.
    let ret = Uint128::new(130_000);
    let mut app = AppBuilder::new().build(|_r, _a, _s| {});
    let user = app.api().addr_make("user");
    let admin = app.api().addr_make("admin");
    app.init_modules(|r, _, s| {
        r.bank
            .init_balance(s, &user, vec![Coin::new(1_000_000u128, ASK)])
            .unwrap();
    });
    let ncode = ninjucks_code(&mut app);
    let mcode = mock_code(&mut app);
    let router = app
        .instantiate_contract(mcode, admin.clone(), &MockInit {}, &[], "r", None)
        .unwrap();
    app.init_modules(|r, _, s| {
        r.bank
            .init_balance(s, &router, vec![Coin::new(10_000_000u128, ASK)])
            .unwrap();
    });
    let nun = app
        .instantiate_contract(
            ncode,
            admin.clone(),
            &InstantiateMsg {
                admin: Some(admin.to_string()),
                choice_router: router.to_string(),
                hallswap_router: router.to_string(),
            },
            &[],
            "nun",
            None,
        )
        .unwrap();

    let before = app.wrap().query_balance(&user, ASK).unwrap().amount;
    app.execute_contract(
        user.clone(),
        nun.clone(),
        &ExecuteMsg::Swap(SwapParams {
            venue: Venue::Choice,
            route: to_json_binary(&MockExec::MockSwap {
                return_denom: ASK.to_string(),
                return_amount: ret,
            })
            .unwrap(),
            ask_denom: ASK.to_string(),
            minimum_receive: Uint128::new(120_000),
            recipient: Some(user.to_string()),
        }),
        &[Coin::new(100_000u128, ASK)],
    )
    .unwrap();
    let after = app.wrap().query_balance(&user, ASK).unwrap().amount;
    // payout is the full 130_000 output, not 130_000 - 100_000 (offer must not be double-counted)
    assert_eq!(after, before - Uint128::new(100_000) + ret);
    // ninjucks holds no residual
    assert_eq!(
        app.wrap().query_balance(&nun, ASK).unwrap().amount,
        Uint128::zero()
    );
}

// ===========================================================================
// 5. AssertAndPay is self-only.
// ===========================================================================
#[test]
fn assert_and_pay_rejects_external_caller() {
    let mut w = setup(Uint128::zero(), 0);
    let err = w.app.execute_contract(
        w.user.clone(),
        w.ninjucks.clone(),
        &ExecuteMsg::AssertAndPay {
            ask_denom: ASK.to_string(),
            minimum_receive: Uint128::zero(),
            recipient: w.user.to_string(),
            balance_before: Uint128::zero(),
        },
        &[],
    );
    assert!(err.is_err(), "external AssertAndPay must be rejected");
}

// ===========================================================================
// 6. Config query + admin gating.
// ===========================================================================
#[test]
fn config_and_admin() {
    let mut w = setup(Uint128::zero(), 0);
    let cfg: Config = w
        .app
        .wrap()
        .query_wasm_smart(w.ninjucks.clone(), &QueryMsg::Config {})
        .unwrap();
    assert_eq!(cfg.choice_router, w.choice);
    assert_eq!(cfg.hallswap_router, w.hallswap);

    // non-admin cannot update
    let err = w.app.execute_contract(
        w.user.clone(),
        w.ninjucks.clone(),
        &ExecuteMsg::UpdateConfig {
            choice_router: Some(w.user.to_string()),
            hallswap_router: None,
        },
        &[],
    );
    assert!(err.is_err(), "non-admin update must be rejected");
}

// ===========================================================================
// 7. Audit-fix coverage: reject minimum_receive == 0.
// ===========================================================================
#[test]
fn zero_minimum_receive_rejected() {
    let ret = Uint128::new(500_000);
    let mut w = setup(ret, 10_000_000);
    let err = w.app.execute_contract(
        w.user.clone(),
        w.ninjucks.clone(),
        &ExecuteMsg::Swap(SwapParams {
            venue: Venue::Choice,
            route: route_for(ret),
            ask_denom: ASK.to_string(),
            minimum_receive: Uint128::zero(),
            recipient: None,
        }),
        &[Coin::new(100_000u128, OFFER)],
    );
    assert!(err.is_err(), "minimum_receive == 0 must be rejected");
}

// 7b. Admin can withdraw stray funds; non-admin cannot.
#[test]
fn withdraw_admin_only_recovers_stray_funds() {
    let mut w = setup(Uint128::zero(), 0);
    let admin = w.app.api().addr_make("admin");
    // strand some OFFER dust in the contract (simulates a router refund of unspent offer)
    w.app.init_modules(|r, _, s| {
        r.bank
            .init_balance(s, &w.ninjucks, vec![Coin::new(777u128, OFFER)])
            .unwrap();
    });
    // non-admin rejected
    assert!(w
        .app
        .execute_contract(
            w.user.clone(),
            w.ninjucks.clone(),
            &ExecuteMsg::Withdraw { denom: OFFER.to_string(), amount: Uint128::new(777), to: None },
            &[],
        )
        .is_err());
    // admin recovers to itself
    w.app
        .execute_contract(
            admin.clone(),
            w.ninjucks.clone(),
            &ExecuteMsg::Withdraw { denom: OFFER.to_string(), amount: Uint128::new(777), to: None },
            &[],
        )
        .unwrap();
    assert_eq!(
        w.app.wrap().query_balance(&admin, OFFER).unwrap().amount,
        Uint128::new(777)
    );
    assert_eq!(
        w.app.wrap().query_balance(&w.ninjucks, OFFER).unwrap().amount,
        Uint128::zero()
    );
}

// 7c. Two-step admin transfer: propose then accept; wrong acceptor rejected.
#[test]
fn two_step_admin_transfer() {
    let mut w = setup(Uint128::zero(), 0);
    let admin = w.app.api().addr_make("admin");
    let next = w.app.api().addr_make("next-admin");

    // only admin can propose
    assert!(w
        .app
        .execute_contract(
            w.user.clone(),
            w.ninjucks.clone(),
            &ExecuteMsg::ProposeAdmin { new_admin: next.to_string() },
            &[],
        )
        .is_err());
    w.app
        .execute_contract(
            admin.clone(),
            w.ninjucks.clone(),
            &ExecuteMsg::ProposeAdmin { new_admin: next.to_string() },
            &[],
        )
        .unwrap();
    // a non-pending party cannot accept
    assert!(w
        .app
        .execute_contract(w.user.clone(), w.ninjucks.clone(), &ExecuteMsg::AcceptAdmin {}, &[])
        .is_err());
    // pending admin accepts
    w.app
        .execute_contract(next.clone(), w.ninjucks.clone(), &ExecuteMsg::AcceptAdmin {}, &[])
        .unwrap();
    let cfg: Config = w
        .app
        .wrap()
        .query_wasm_smart(w.ninjucks.clone(), &QueryMsg::Config {})
        .unwrap();
    assert_eq!(cfg.admin, next);
    assert_eq!(cfg.pending_admin, None);
    // old admin no longer authorized
    assert!(w
        .app
        .execute_contract(
            admin.clone(),
            w.ninjucks.clone(),
            &ExecuteMsg::UpdateConfig { choice_router: Some(w.user.to_string()), hallswap_router: None },
            &[],
        )
        .is_err());
}
