# Ninjucks — design

A unified swap **aggregator** for Injective. One contract, one interface, over multiple DEX venues
(Choice and HallSwap today). It holds no liquidity — it routes an incoming swap through the chosen
underlying router, enforces the caller's minimum-receive, and pays out the result.

## Entrypoints

Ninjucks exposes the same routing logic through two entrypoints, both thin wrappers around one
internal `do_swap`:

- `Swap` — the ordinary contract entrypoint. The offer coin is attached as funds.
- `injective_exec` — the envelope form `{"injective_exec": {origin, name, args}}`; `args` decodes to
  the same swap parameters. The offer coin arrives as `info.funds`, and `origin` is the caller of
  record used to default the recipient.

Routing, the slippage check, and payout are identical for both.

## Funds flow

`do_swap`:
1. `offer = info.funds[0]`, `recipient = msg.recipient.unwrap_or(caller)`.
2. Reject `minimum_receive == 0` (it would disable the slippage guard).
3. Snapshot the contract's `ask_denom` balance as a baseline (excluding the offer when
   `offer_denom == ask_denom`, since that offer is about to be spent).
4. Dispatch **one** sub-message to the chosen venue's router (`WasmMsg::Execute`, offer as funds).
   The `route` body is forwarded verbatim; the caller is responsible for directing the router's
   output back to the Ninjucks contract. If output is misrouted, the balance delta is zero and the
   tx reverts — so mis-routing is safe, never a loss.
5. An ordered self-message `AssertAndPay` runs after the swap settles: it computes
   `received = balance_now − baseline`, requires `received > 0` and `received >= minimum_receive`
   (else the whole tx reverts atomically), and `BankMsg::Send`s `received` to `recipient`.

A swap returns no response `data`.

## Venue dispatch (v1)

v1 routes a swap through exactly one venue per call, chosen by the caller. The caller supplies the
venue-native route body (`route: Binary`), forwarded verbatim as the sub-router's execute payload —
so legs are never re-encoded and each router's exact semantics are inherited. (A future v2 can add
multi-venue splitting and a native venue-tagged route type.)

## Security model

- **Non-custodial per swap.** Each swap fully settles within its own transaction: `AssertAndPay`
  sweeps the entire ask-denom delta to `recipient`, leaving the ask baseline untouched. A non-ask
  residual (e.g. a router refunding unspent offer on a normal swap, or a stray transfer) is **not**
  stealable — the per-denom baseline always contains it — but it is stuck, so an admin-gated
  `Withdraw` recovers it. Same-denom cycles leave no residual.
- **Atomic slippage guard.** `received > 0` and `received >= minimum_receive`, or the tx reverts.
  `minimum_receive == 0` is rejected — it is the sole guarantee that output actually returned.
- **`AssertAndPay` is self-only.** Callable exclusively by the contract address.
- **No arbitrary-message passthrough.** Entrypoints accept a *swap route*, not a generic message.
  The only sub-messages Ninjucks can emit are a swap to a **configured** router, a payout
  `BankMsg::Send`, and an admin `Withdraw`. The sub-call target is always a configured router.
- **Admin.** Router updates + `Withdraw` are admin-gated; admin transfer is two-step
  (`ProposeAdmin` + `AcceptAdmin`). Routes themselves are permissionless.
- **Upgradability.** A `migrate` entrypoint (gated by the on-chain contract admin) allows in-place
  patching; a `cw2` version is stored.
- **Reentrancy.** The balance-diff accounting is snapshot-based (serialized by value into the
  self-only `AssertAndPay`) and the payout sweeps the actual delta, so a reentrant sub-call cannot
  inflate payout beyond real received funds.

## Config

```
Config { admin, pending_admin, choice_router, hallswap_router }
```

## Out of scope (v1)

- Multi-venue split routing (v2).
- Native leg encoding / best-path selection inside the contract (v2; today the caller/off-chain path
  builder chooses the venue and route body).
- Fees. v1 takes no protocol fee; a `fee_bps`/`fee_collector` can be added without changing the
  funds-flow model.
