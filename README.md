# Jupiter AMM interface

A Cargo workspace for integrating a Solana DEX into Jupiter routing.

| Crate | What it is |
|---|---|
| [`interface/`](interface) | `jupiter-amm-interface` — the `Amm` trait a DEX implements to be routed by Jupiter. |
| [`test-kit/`](test-kit) | `jupiter-amm-test-kit` — a harness to verify an SDK's `Amm::quote()` matches the on-chain program (snapshot a pool, run the quote, execute the native swap in LiteSVM, assert parity) before handing the SDK to Jupiter. |

## Getting started

1. Implement `Amm` for the DEX against [`interface/`](interface).
2. Verify it against the real program with [`test-kit/`](test-kit) — see its README for the
   test-suite pattern.

Questions? Ask in the [Jupiter Discord](https://discord.gg/jup) `#developer-support` channel.
