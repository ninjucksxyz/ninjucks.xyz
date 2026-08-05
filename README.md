# Ninjucks

[ninjucks.xyz](https://ninjucks.xyz)

**A unified swap aggregator for Injective.** One interface to route swaps across Injective's DEX
liquidity — Choice and HallSwap today, with room for more venues.

Ninjucks holds no liquidity of its own. It takes an incoming swap, routes it through the chosen
venue, enforces your minimum-receive, and pays the output to the recipient in the same transaction.
If the swap can't meet your minimum, the whole transaction reverts.

## Features

- **One message, many venues** — stop special-casing each router's schema.
- **Non-custodial & atomic** — funds are never parked in the contract; every swap fully settles or
  reverts.
- **Slippage-safe** — a `minimum_receive` floor is enforced by the aggregator itself.

## Interface

```jsonc
{ "swap": {
    "venue": "hallswap",              // or "choice"
    "route": "<base64 venue-native route body>",
    "ask_denom": "inj",
    "minimum_receive": "1000000",
    "recipient": null                  // defaults to the caller
} }
```

The `route` is the venue's own route object, forwarded verbatim, so Ninjucks inherits each venue's
exact swap semantics.

## Repository

- `src/` — the aggregator contract (CosmWasm)
- `tests/` — contract tests · `testkit/` — a mock router used in tests
- `frontend/` — a minimal web UI (`build/` bundles the SDK it uses)

## Reproducible build

The contract is built with the standard `cosmwasm/optimizer` and the committed `Cargo.lock`, so the
hash is deterministic — anyone can reproduce it:

```
docker run --rm -v "$PWD":/code \
  --mount type=volume,source=nj_cache,target=/target \
  --mount type=volume,source=nj_registry,target=/usr/local/cargo/registry \
  cosmwasm/optimizer:0.17.0
```

```
ninjucks.wasm  SHA256  0c983abbc0373b28d4a7aa5a25fe059d36dc13604459a97c3a1ca1b01e888bb4
```

The governance proposal uploads this exact wasm; the on-chain `code data_hash` equals the SHA256
above (and the hash in the proposal). CI rebuilds it and fails on any mismatch.

## Status

Early software — read `DESIGN.md` for the architecture and security model, and the source before use.

## License

Apache-2.0.
