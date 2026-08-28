# jupiter-amm-interface

Use the `jupiter-amm-interface` crate to implement an `Amm` for a Solana DEX so it can be
routed by Jupiter.

More information: [DEX integration guidelines](https://developers.jup.ag/docs/swap/routing/dex-integration#integrate-dex-into-metis)

Questions? Ask in the [Jupiter Discord](https://discord.gg/jup) `#developer-support` channel.

## Solana types come from here

An `Amm` implementation needs **no `solana-*` dependencies of its own**. Every Solana type
in the trait is re-exported under `jupiter_amm_interface::solana`, laid out like the old
monolithic `solana_program`:

| Path | Crate | What you need it for |
|---|---|---|
| `solana::pubkey` | `solana-pubkey` | `Pubkey`, the `pubkey!` macro |
| `solana::account` | `solana-account` | `Account`, `ReadableAccount` |
| `solana::instruction` | `solana-instruction` | `AccountMeta`, `Instruction` |
| `solana::program_error` | `solana-program-error` | `ProgramError` |
| `solana::clock` | `solana-clock` | `Clock` (for `ClockRef`) |

```rust
use jupiter_amm_interface::solana::{
    account::ReadableAccount,
    instruction::AccountMeta,
    pubkey::{Pubkey, pubkey},
};
```

The two non-Solana crates in the public API are re-exported at the root for the same
reason: `rust_decimal` (`Quote::fee_pct`) and `serde_json` (`KeyedAccount::params`).

Taking the types from here rather than declaring your own `solana-*` deps is what keeps a
DEX SDK on a single Solana crate generation. Two generations in one tree give you two
distinct `Pubkey` types that don't unify, and the errors that follow ("expected `Pubkey`,
found `Pubkey`") are the most common thing that stalls an integration.

If your SDK needs a Solana crate that isn't re-exported here,
[open an issue](https://github.com/jup-ag/jupiter-amm-interface/issues) rather than adding
your own — we would rather re-export it.

## Verifying an integration

After implementing `Amm`, use [`jupiter-amm-test-kit`](../test-kit) to prove the
`quote()` matches the on-chain program before handing the SDK to Jupiter.
