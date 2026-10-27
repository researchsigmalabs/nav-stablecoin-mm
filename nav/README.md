# nav

Mark-to-pool NAV computation for the **Stablecoin Market Making** vault (sigmaLPP) — `compute_nav`
(idle + Σ liquidity principal + Σ uncollected fees + Σ tokensOwed, per token, valued at the pool
price) + the `nav_is_sane` guard. Pure: no network, no secrets, no keys; it takes an
`InventorySnapshot` of on-chain reads (pool state, the vault's active V3 positions, idle balances)
and returns the total NAV in the underlying (USDC, 6-decimal) units.

The Uniswap V3 math (`get_sqrt_ratio_at_tick`, `get_amounts_for_liquidity`, fee-growth) is a direct
integer port; its tests check it against Uniswap's published tick constants and reconcile the
principal and `value_in_token_0` against a real on-chain snapshot to the wei.

See [`../docs/nav.md`](../docs/nav.md) for the methodology — how each leg is sourced on-chain so the
figure is independently reproducible.

```sh
cargo test
```

Apache-2.0 © σ-Labs
