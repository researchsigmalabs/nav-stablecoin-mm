# Stablecoin Market Making

Open reference material for the σ-Labs / Ellen Capital **Stablecoin Market Making** Lagoon vault
(**sigmaLPP**).

The strategy market-makes the USDC/USDT 0.01% pool on Uniswap V3, in partnership with
[Lobster](https://www.lobster-protocol.com): it places concentrated liquidity around the peg and
rebalances as the price drifts, earning swap fees on a stable pair. This repository publishes the
parts of the keeper we open-source so the vault is independently verifiable — starting with the
**NAV computation**.

- **Live dashboard:** https://www.sigmalabs.fi/vault/0x8595f5ee036e1f7c4f3ca66327ba5f38c21cb7fb
- **Lagoon vault:** [`0x8595…b7fb`](https://app.lagoon.finance/vault/1/0x8595f5ee036e1f7c4f3ca66327ba5f38c21cb7fb)
- **Pool:** [`0x3416…27C6`](https://etherscan.io/address/0x3416cF6C708Da44DB2624D63ea0AAef7113527C6) (USDC/USDT, 0.01%)

## Layout

| Path | What |
|------|------|
| [`nav/`](./nav) | The mark-to-pool NAV computation crate — the exact Uniswap V3 valuation math the keeper submits on-chain (`compute_nav` + the `nav_is_sane` guard, with tests reconciled against real on-chain data). Pure: no network, no secrets, no keys. |
| [`docs/`](./docs) | Methodology. [`docs/nav.md`](./docs/nav.md) documents how every NAV leg is sourced on-chain so the figure is independently reproducible. |

## License

[Apache-2.0](./LICENSE) © σ-Labs
