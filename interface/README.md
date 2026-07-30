# jupiter-amm-interface

Use the `jupiter-amm-interface` crate to implement an `Amm` for a Solana DEX so it can be
routed by Jupiter.

More information: [DEX integration guidelines](https://developers.jup.ag/docs/swap/routing/dex-integration#integrate-dex-into-metis)

Questions? Ask in the [Jupiter Discord](https://discord.gg/jup) `#developer-support` channel.

## Verifying an integration

After implementing `Amm`, use [`jupiter-amm-test-kit`](../test-kit) to prove the
`quote()` matches the on-chain program before handing the SDK to Jupiter.
