# NAV methodology

How the **Stablecoin Market Making** vault's (sigmaLPP) Net Asset Value (NAV) is computed and pushed
on-chain. The math lives in [`../nav`](../nav) and is fully reproducible from public on-chain state.

## The model

The vault makes markets on **one** Uniswap V3 pool — USDC/USDT, 0.01% fee
([`0x3416…27C6`](https://etherscan.io/address/0x3416cF6C708Da44DB2624D63ea0AAef7113527C6), token0 =
USDC, token1 = USDT). Its NAV is the mark-to-market value of everything it holds, valued **at the
pool price** (`slot0.sqrtPriceX96`) — not at par. Per token:

```
NAV_token0 = balanceOf(safe, USDC)
           + Σ getAmount0(liquidity, tickLower, tickUpper, sqrtPriceX96)   // principal
           + Σ uncollectedFee0(liquidity, feeGrowthInside0 − checkpoint)
           + Σ tokensOwed0

NAV_token1 = balanceOf(safe, USDT) + Σ getAmount1(…) + Σ uncollectedFee1(…) + Σ tokensOwed1
```

and the vault's NAV in its underlying (USDC) is the two legs combined at the pool price
(`compute_nav`):

```
NAV = NAV_token0 + NAV_token1 · 2^192 / sqrtPriceX96^2
```

Valuing the USDT leg at the pool price rather than at par is deliberate: if USDT depegs, the pool
price moves and the arbitrageurs rotate the position toward USDT, so a pool-price mark reflects the
real value of the book instead of masking it. The pool is the oracle; no external price feed is used.

Everything is a `bigint` in raw on-chain wei (fixed-point products widen to 512 bits so nothing
overflows). Divide by `10^decimals` only at display time.

## How each leg is sourced on-chain

Every leg is a direct on-chain read, so anyone can reproduce the NAV. The strategy Safe
([`0xd605…36E3`](https://etherscan.io/address/0xd6054103F0ADF95e98D01C3d252E6Ad07c2736E3)) is the
owner of the V3 positions.

| Leg | On-chain read |
|-----|---------------|
| active ranges | `eth_getLogs` on the pool's `Mint`/`Burn` events filtered by `owner = safe`; a range is active where net minted liquidity > 0. `positionKey = keccak256(abi.encodePacked(safe, tickLower, tickUpper))` |
| `idle` | `USDC.balanceOf(safe)` and `USDT.balanceOf(safe)` |
| `principal` (per range) | `get_amounts_for_liquidity(slot0.sqrtPriceX96, sqrtRatioAtTick(tickLower), sqrtRatioAtTick(tickUpper), positions(key).liquidity)` |
| `uncollected fees` (per range) | `(feeGrowthInside − positions(key).feeGrowthInside{0,1}LastX128) · liquidity / 2^128`, where `feeGrowthInside` is derived from `feeGrowthGlobal{0,1}X128` and `ticks(tickLower/tickUpper).feeGrowthOutside{0,1}X128` (Uniswap's wraparound accounting) |
| `tokensOwed` (per range) | `positions(key).tokensOwed0` / `tokensOwed1` — fees already collected into the position, awaiting withdrawal |
| pool price | `slot0.sqrtPriceX96` and `slot0.tick` |

The Uniswap V3 integer math (`get_sqrt_ratio_at_tick`, `get_amounts_for_liquidity`, the fee-growth
functions) is a direct port of the pool's own arithmetic. The crate's tests check `get_sqrt_ratio_at_tick`
against Uniswap's published `MIN_SQRT_RATIO` / `MAX_SQRT_RATIO` constants, and reconcile the computed
`principal` and `value_in_token_0` against a real on-chain snapshot to the wei. The same figures are
served, as a convenience, by Lobster's public `GET /api/nav/{live,history}` endpoints — compare
against them at a pinned block for an independent check.

## The sanity guard

`nav_is_sane` runs before every on-chain submission. It rejects:

- a **zero NAV while shares exist** (`totalSupply > 0`), and
- any move larger than `max_jump_pct` versus the previous on-chain `totalAssets`.

A wrong NAV mis-prices every share, so this guard is mandatory. The comparison is integer-only (no
float rounding).

## Price manipulation

The NAV is marked at the pool's **spot** price (`slot0.sqrtPriceX96`), which an attacker can move
within a single block. On a deep 1bp stablecoin pool this is costly and self-correcting, but the
keeper does not rely on that alone. Before each submission it:

1. reads a **TWAP** from `pool.observe(...)` over a sensible window and calls `pool_price_is_undisturbed(spot, twap, max_bps)`, rejecting the snapshot when spot deviates from the TWAP by more than a few basis points;
2. sets a **tight** `max_jump_pct` on `nav_is_sane` (a stablecoin NAV should barely move between settles);
3. relies on the **on-chain Lagoon guardrail** (`updateGuardrails`) as the final bound on how far the submitted NAV may move per settle.

Marking at the pool price is deliberate — it reflects a real USDT depeg instead of masking it — so the
defense is to guard the *price the mark uses*, not to switch to an off-pool oracle.

## On-chain submission

This repository is **only the math**. On-chain, the vault's valuation-provider role (Admin Safe
[`0xEd93…bEbc1`](https://etherscan.io/address/0xEd93F0ED8F3cF989806c1A7AFb223870D83bEbc1)) submits the
NAV via Lagoon's `updateNewTotalAssets`, and the settlement is validated before each
deposit/redemption cycle (roughly twice daily). The keeper reaches the vault only through
narrowly-scoped Zodiac Roles v2 modules; it holds no custody. The strategy Safe
([`0xd605…36E3`](https://etherscan.io/address/0xd6054103F0ADF95e98D01C3d252E6Ad07c2736E3)) operates the
liquidity on the pool.
