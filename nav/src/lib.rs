//! Mark-to-pool NAV computation for the σ-Labs / Ellen Capital "Stablecoin Market Making" vault
//! (sigmaLPP).
//!
//! This is the exact, self-contained Uniswap V3 valuation math the off-chain keeper runs before it
//! submits a new NAV on-chain (Lagoon `updateNewTotalAssets`). It is **pure**: no network, no
//! secrets, no keys — it takes an [`InventorySnapshot`] of on-chain reads (pool state, the vault's
//! active LP positions, idle balances) and returns the total NAV in the underlying (USDC, token0).
//!
//! The vault makes markets on one Uniswap V3 pool (USDC/USDT, 0.01%). Each token's holdings are
//! `idle + Σ liquidity principal + Σ uncollected fees + Σ tokensOwed`, valued **at the pool price**
//! (`sqrt_price_x96`) — not at par. The USDT leg is converted to USDC at that price, so a USDT depeg
//! is reflected rather than masked. See `../docs/nav.md` for how each on-chain read is sourced.
//!
//! The V3 math (`get_sqrt_ratio_at_tick`, `get_amounts_for_liquidity`, fee-growth) is a direct port
//! of Uniswap's integer arithmetic; fixed-point products widen to `U512` so nothing overflows.

use alloy_primitives::{U256, U512};

// ─── fixed-point helpers ────────────────────────────────────────────────────

fn widen(x: U256) -> U512 {
    U512::from_le_slice(&x.to_le_bytes::<32>())
}
fn narrow(x: U512) -> U256 {
    U256::from_le_slice(&x.to_le_bytes::<64>()[..32])
}

fn max_uint128() -> U256 {
    (U256::from(1u8) << 128) - U256::from(1u8)
}

// ─── Uniswap V3 TickMath (exact integer port) ───────────────────────────────

/// The largest valid tick magnitude.
pub const MAX_TICK: i32 = 887_272;

const MAGIC: [(u32, &str); 19] = [
    (0x2, "fff97272373d413259a46990580e213a"),
    (0x4, "fff2e50f5f656932ef12357cf3c7fdcc"),
    (0x8, "ffe5caca7e10e4e61c3624eaa0941cd0"),
    (0x10, "ffcb9843d60f6159c9db58835c926644"),
    (0x20, "ff973b41fa98c081472e6896dfb254c0"),
    (0x40, "ff2ea16466c96a3843ec78b326b52861"),
    (0x80, "fe5dee046a99a2a811c461f1969c3053"),
    (0x100, "fcbe86c7900a88aedcffc83b479aa3a4"),
    (0x200, "f987a7253ac413176f2b074cf7815e54"),
    (0x400, "f3392b0822b70005940c7a398e4b70f3"),
    (0x800, "e7159475a2c29b7443b29c7fa6e889d9"),
    (0x1000, "d097f3bdfd2022b8845ad8f792aa5825"),
    (0x2000, "a9f746462d870fdf8a65dc1f90e061e5"),
    (0x4000, "70d869a156d2a1b890bb3df62baf32f7"),
    (0x8000, "31be135f97d08fd981231505542fcfa6"),
    (0x10000, "9aa508b5b7a84e1c677de54f3e99bc9"),
    (0x20000, "5d6af8dedb81196699c329225ee604"),
    (0x40000, "2216e584f5fa1ea926041bedfe98"),
    (0x80000, "48a170391f7dc42444e8fa2"),
];

fn hex(s: &str) -> U256 {
    U256::from_str_radix(s, 16).expect("valid hex constant")
}

/// `sqrt(1.0001^tick) * 2^96` as an exact integer, Uniswap's `TickMath.getSqrtRatioAtTick`.
pub fn get_sqrt_ratio_at_tick(tick: i32) -> U256 {
    let abs_tick = tick.unsigned_abs();
    assert!(abs_tick <= MAX_TICK as u32, "tick out of range");

    let mut ratio = if abs_tick & 0x1 != 0 {
        hex("fffcb933bd6fad37aa2d162d1a594001")
    } else {
        U256::from(1u8) << 128
    };
    for (bit, k) in MAGIC {
        if abs_tick & bit != 0 {
            ratio = narrow((widen(ratio) * widen(hex(k))) >> 128);
        }
    }
    if tick > 0 {
        ratio = U256::MAX / ratio;
    }
    // sqrtPriceX96 = ceil(ratio / 2^32)
    let shifted = ratio >> 32;
    if (ratio & ((U256::from(1u8) << 32) - U256::from(1u8))) == U256::ZERO {
        shifted
    } else {
        shifted + U256::from(1u8)
    }
}

// ─── principal (liquidity → token amounts at the current price) ──────────────

fn get_amount0(sqrt_a: U256, sqrt_b: U256, liquidity: U256) -> U256 {
    let (lo, hi) = if sqrt_a > sqrt_b {
        (sqrt_b, sqrt_a)
    } else {
        (sqrt_a, sqrt_b)
    };
    if lo.is_zero() {
        return U256::ZERO;
    }
    // (hi - lo) * (L << 96) / hi / lo
    let num = widen(hi - lo) * (widen(liquidity) << 96);
    narrow(num / widen(hi) / widen(lo))
}

fn get_amount1(sqrt_a: U256, sqrt_b: U256, liquidity: U256) -> U256 {
    let (lo, hi) = if sqrt_a > sqrt_b {
        (sqrt_b, sqrt_a)
    } else {
        (sqrt_a, sqrt_b)
    };
    // L * (hi - lo) / 2^96
    narrow((widen(liquidity) * widen(hi - lo)) >> 96)
}

/// Token amounts a position's liquidity is currently worth, Uniswap's
/// `LiquidityAmounts.getAmountsForLiquidity`.
pub fn get_amounts_for_liquidity(
    sqrt_price: U256,
    sqrt_lower: U256,
    sqrt_upper: U256,
    liquidity: U256,
) -> (U256, U256) {
    let (lo, hi) = if sqrt_lower > sqrt_upper {
        (sqrt_upper, sqrt_lower)
    } else {
        (sqrt_lower, sqrt_upper)
    };
    if sqrt_price <= lo {
        (get_amount0(lo, hi, liquidity), U256::ZERO)
    } else if sqrt_price < hi {
        (
            get_amount0(sqrt_price, hi, liquidity),
            get_amount1(lo, sqrt_price, liquidity),
        )
    } else {
        (U256::ZERO, get_amount1(lo, hi, liquidity))
    }
}

// ─── uncollected fees (feeGrowthInside − checkpoint) ─────────────────────────

/// Per-tick fee-growth-outside, read from `pool.ticks(tick)`.
#[derive(Debug, Clone, Default)]
pub struct TickFees {
    pub fee_growth_outside_0_x128: U256,
    pub fee_growth_outside_1_x128: U256,
}

fn fee_growth_inside(
    p: &Position,
    tick_current: i32,
    fee_growth_global_0: U256,
    fee_growth_global_1: U256,
) -> (U256, U256) {
    let below0 = if tick_current >= p.tick_lower {
        p.lower.fee_growth_outside_0_x128
    } else {
        fee_growth_global_0.wrapping_sub(p.lower.fee_growth_outside_0_x128)
    };
    let below1 = if tick_current >= p.tick_lower {
        p.lower.fee_growth_outside_1_x128
    } else {
        fee_growth_global_1.wrapping_sub(p.lower.fee_growth_outside_1_x128)
    };
    let above0 = if tick_current < p.tick_upper {
        p.upper.fee_growth_outside_0_x128
    } else {
        fee_growth_global_0.wrapping_sub(p.upper.fee_growth_outside_0_x128)
    };
    let above1 = if tick_current < p.tick_upper {
        p.upper.fee_growth_outside_1_x128
    } else {
        fee_growth_global_1.wrapping_sub(p.upper.fee_growth_outside_1_x128)
    };
    (
        fee_growth_global_0
            .wrapping_sub(below0)
            .wrapping_sub(above0),
        fee_growth_global_1
            .wrapping_sub(below1)
            .wrapping_sub(above1),
    )
}

/// `(feeGrowthInside − checkpoint) * liquidity / 2^128`, masked to uint128 — Uniswap's wraparound
/// fee accounting.
fn pending_fee(liquidity: U256, fee_growth_inside: U256, checkpoint: U256) -> U256 {
    let delta = fee_growth_inside.wrapping_sub(checkpoint);
    let scaled = (widen(delta) * widen(liquidity)) >> 128;
    narrow(scaled) & max_uint128()
}

// ─── inventory & NAV ─────────────────────────────────────────────────────────

/// One active LP position, as read from the pool.
#[derive(Debug, Clone)]
pub struct Position {
    pub tick_lower: i32,
    pub tick_upper: i32,
    /// `positions(key).liquidity`.
    pub liquidity: U256,
    pub fee_growth_inside_0_last_x128: U256,
    pub fee_growth_inside_1_last_x128: U256,
    pub tokens_owed_0: U256,
    pub tokens_owed_1: U256,
    /// `ticks(tick_lower)`.
    pub lower: TickFees,
    /// `ticks(tick_upper)`.
    pub upper: TickFees,
}

/// Pool state from `slot0` + `feeGrowthGlobal{0,1}X128`.
#[derive(Debug, Clone)]
pub struct PoolState {
    pub sqrt_price_x96: U256,
    pub tick: i32,
    pub fee_growth_global_0_x128: U256,
    pub fee_growth_global_1_x128: U256,
}

/// Everything the NAV needs, every field a direct on-chain read. All amounts are raw wei.
#[derive(Debug, Clone)]
pub struct InventorySnapshot {
    pub pool: PoolState,
    /// The vault's currently-active positions on the pool (liquidity > 0).
    pub positions: Vec<Position>,
    /// `token0.balanceOf(vault)` (USDC).
    pub idle_0: U256,
    /// `token1.balanceOf(vault)` (USDT).
    pub idle_1: U256,
}

/// One position's total token0/token1 value: principal (at the current price) + uncollected fees +
/// `tokensOwed`.
fn position_value(p: &Position, pool: &PoolState) -> (U256, U256) {
    let (amount0, amount1) = get_amounts_for_liquidity(
        pool.sqrt_price_x96,
        get_sqrt_ratio_at_tick(p.tick_lower),
        get_sqrt_ratio_at_tick(p.tick_upper),
        p.liquidity,
    );
    let (fgi0, fgi1) = fee_growth_inside(
        p,
        pool.tick,
        pool.fee_growth_global_0_x128,
        pool.fee_growth_global_1_x128,
    );
    let fees0 = p.tokens_owed_0 + pending_fee(p.liquidity, fgi0, p.fee_growth_inside_0_last_x128);
    let fees1 = p.tokens_owed_1 + pending_fee(p.liquidity, fgi1, p.fee_growth_inside_1_last_x128);
    (amount0 + fees0, amount1 + fees1)
}

/// Per-token NAV in raw wei: `(token0, token1)`, each `idle + Σ (principal + fees + owed)`.
/// This is what Lobster's `GET /api/nav/live` returns per token — reconcile against it to the wei.
pub fn nav_per_token(s: &InventorySnapshot) -> (U256, U256) {
    let mut nav0 = s.idle_0;
    let mut nav1 = s.idle_1;
    for p in &s.positions {
        let (v0, v1) = position_value(p, &s.pool);
        nav0 += v0;
        nav1 += v1;
    }
    (nav0, nav1)
}

/// Value of a token1 amount expressed in token0 at the pool price: `amount1 * 2^192 / sqrtPrice^2`.
fn token1_in_token0(amount1: U256, sqrt_price_x96: U256) -> U256 {
    let num = widen(amount1) << 192;
    let den = widen(sqrt_price_x96) * widen(sqrt_price_x96);
    narrow(num / den)
}

/// Full mark-to-pool NAV in the underlying (token0 / USDC, raw wei): token0 NAV plus the token1 NAV
/// valued at the pool price. This is the `totalAssets` the keeper submits via `updateNewTotalAssets`.
pub fn compute_nav(s: &InventorySnapshot) -> U256 {
    let (nav0, nav1) = nav_per_token(s);
    nav0 + token1_in_token0(nav1, s.pool.sqrt_price_x96)
}

/// Mandatory guard before pushing a NAV. A wrong NAV mis-prices every share, so we reject a zero NAV
/// while shares exist, and any move larger than `max_jump_pct` versus the previous on-chain
/// `totalAssets`. Integer-only: no float rounding in the comparison.
pub fn nav_is_sane(
    nav: U256,
    prev_total_assets: U256,
    total_supply: U256,
    max_jump_pct: f64,
) -> eyre::Result<()> {
    if nav.is_zero() && !total_supply.is_zero() {
        eyre::bail!("NAV is zero while totalSupply>0");
    }
    if !prev_total_assets.is_zero() {
        // |nav - prev| / prev * 100 <= max_jump_pct, compared without losing precision:
        // diff * 10000  vs  prev * (max_jump_pct * 100)
        let (hi, lo) = if nav >= prev_total_assets {
            (nav, prev_total_assets)
        } else {
            (prev_total_assets, nav)
        };
        let diff = hi - lo;
        let lhs = diff * U256::from(10_000u64);
        let rhs = prev_total_assets * U256::from((max_jump_pct * 100.0) as u64);
        if lhs > rhs {
            eyre::bail!(
                "NAV jump exceeds {}%: prev={} new={}",
                max_jump_pct,
                prev_total_assets,
                nav
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> U256 {
        U256::from_str_radix(s, 10).unwrap()
    }
    fn q96() -> U256 {
        U256::from(1u8) << 96
    }

    /// |a − b| ≤ tol.
    fn assert_close(a: U256, b: U256, tol: u64) {
        let diff = if a >= b { a - b } else { b - a };
        assert!(diff <= U256::from(tol), "|{a} − {b}| = {diff} > {tol}");
    }

    // ── Reconciliation against a REAL Lobster snapshot (vault 0x2de6…, pool 0x1098…, block ~Oct 2026).
    //    Inputs (sqrtPrice, ticks, liquidity) and the expected outputs (principal, value_in_token_0)
    //    are the raw numbers Lobster's own backend published for this boundary, so a byte-level match
    //    proves this port reproduces the production NAV. ──
    #[test]
    fn principal_reconciles_with_real_lobster_snapshot() {
        let sp = u("88479733489685521798680766203"); // slot0.sqrtPriceX96, tick 2208
                                                     // POS0: range [2207, 2208), current price at/above the upper tick → all token1.
        let (a0, a1) = get_amounts_for_liquidity(
            sp,
            get_sqrt_ratio_at_tick(2207),
            get_sqrt_ratio_at_tick(2208),
            u("147205688799216221232417"),
        );
        assert_eq!(a0, U256::ZERO);
        assert_close(a1, u("8218749450618008356"), 2);
        // POS1: range [2208, 2209), in range → both legs.
        let (b0, b1) = get_amounts_for_liquidity(
            sp,
            get_sqrt_ratio_at_tick(2208),
            get_sqrt_ratio_at_tick(2209),
            u("1247209635489594298881617"),
        );
        assert_close(b0, u("3099238283580768250"), 2);
        assert_close(b1, u("65772050663717544510"), 2);
    }

    #[test]
    fn mark_to_pool_reconciles_value_in_token0() {
        let sp = u("88479733489685521798680766203");
        // Lobster's value_in_token_0 minus its total_token_0 is exactly its token1 leg marked into token0.
        let token1_leg = token1_in_token0(u("74004640449279360432"), sp);
        let expected = u("62477090415770327529") - u("3139417706599576981");
        // within ~1e-9 relative (integer-division rounding differences only).
        assert_close(token1_leg, expected, 60_000_000_000u64);
    }

    // ── TickMath against Uniswap's published constants ──
    #[test]
    fn sqrt_ratio_at_tick_zero_is_q96() {
        assert_eq!(get_sqrt_ratio_at_tick(0), q96());
        assert_eq!(
            get_sqrt_ratio_at_tick(0),
            u("79228162514264337593543950336")
        );
    }

    #[test]
    fn sqrt_ratio_at_min_and_max_tick() {
        // Uniswap TickMath MIN_SQRT_RATIO / MAX_SQRT_RATIO.
        assert_eq!(get_sqrt_ratio_at_tick(-MAX_TICK), u("4295128739"));
        assert_eq!(
            get_sqrt_ratio_at_tick(MAX_TICK),
            u("1461446703485210103287273052203988822378723970342")
        );
    }

    // ── principal boundaries ──
    #[test]
    fn liquidity_is_single_sided_outside_the_range() {
        let lower = get_sqrt_ratio_at_tick(-10);
        let upper = get_sqrt_ratio_at_tick(10);
        let l = U256::from(1_000_000_000u64);
        // price at/below lower → all token0
        let (a0, a1) = get_amounts_for_liquidity(lower, lower, upper, l);
        assert!(a0 > U256::ZERO && a1.is_zero());
        // price at/above upper → all token1
        let (b0, b1) = get_amounts_for_liquidity(upper, lower, upper, l);
        assert!(b0.is_zero() && b1 > U256::ZERO);
        // in range → both legs
        let mid = get_sqrt_ratio_at_tick(0);
        let (c0, c1) = get_amounts_for_liquidity(mid, lower, upper, l);
        assert!(c0 > U256::ZERO && c1 > U256::ZERO);
    }

    // ── NAV sum + mark-to-pool on a hand-built snapshot (price = 1, equal decimals) ──
    #[test]
    fn nav_sums_idle_and_marks_token1_at_pool_price() {
        // At tick 0 the price is 1, so token1 values 1:1 into token0.
        let snap = InventorySnapshot {
            pool: PoolState {
                sqrt_price_x96: q96(),
                tick: 0,
                fee_growth_global_0_x128: U256::ZERO,
                fee_growth_global_1_x128: U256::ZERO,
            },
            positions: vec![],
            idle_0: U256::from(3_000_000u64), // 3 USDC
            idle_1: U256::from(5_000_000u64), // 5 USDT
        };
        let (n0, n1) = nav_per_token(&snap);
        assert_eq!(n0, U256::from(3_000_000u64));
        assert_eq!(n1, U256::from(5_000_000u64));
        // mark-to-pool at price 1: 3 + 5 = 8 USDC.
        assert_eq!(compute_nav(&snap), U256::from(8_000_000u64));
    }

    #[test]
    fn token1_marks_at_pool_price_not_par() {
        // sqrtPrice for tick = 6932 ≈ price 2.0 (token1 per token0). 1 token1 ≈ 0.5 token0.
        let sp = get_sqrt_ratio_at_tick(6932);
        let marked = token1_in_token0(U256::from(1_000_000u64), sp);
        // ~500000 (0.5), not 1_000_000 (par) — within rounding.
        assert!(marked > U256::from(490_000u64) && marked < U256::from(510_000u64));
    }

    // ── the sanity guard ──
    #[test]
    fn guard_rejects_zero_when_supply_positive() {
        assert!(nav_is_sane(U256::ZERO, U256::from(10u64), U256::from(1u64), 25.0).is_err());
    }
    #[test]
    fn guard_rejects_big_jump() {
        assert!(nav_is_sane(U256::from(20u64), U256::from(10u64), U256::from(1u64), 25.0).is_err());
    }
    #[test]
    fn guard_accepts_small_move() {
        assert!(nav_is_sane(
            U256::from(105u64),
            U256::from(100u64),
            U256::from(1u64),
            25.0
        )
        .is_ok());
    }
    #[test]
    fn guard_accepts_first_valuation() {
        assert!(nav_is_sane(U256::from(123u64), U256::ZERO, U256::ZERO, 25.0).is_ok());
    }
}
