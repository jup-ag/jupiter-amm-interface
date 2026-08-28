# Jupiter AMM interface

A Cargo workspace for integrating a Solana DEX into Jupiter routing.

| Crate | What it is |
|---|---|
| [`interface/`](interface) | `jupiter-amm-interface` — the `Amm` trait a DEX implements to be routed by Jupiter. |
| [`test-kit/`](test-kit) | `jupiter-amm-test-kit` — a harness to verify an SDK's `Amm::quote()` matches the on-chain program (snapshot a pool, run the quote, execute the native swap in LiteSVM, assert parity) before handing the SDK to Jupiter. |

## Getting started

1. Implement `Amm` for the DEX against [`interface/`](interface). Take every Solana type
   from `jupiter_amm_interface::solana` rather than adding `solana-*` dependencies of your
   own — that is what keeps the SDK on one Solana crate generation.
2. Verify it against the real program with [`test-kit/`](test-kit) — see its README for the
   test-suite pattern, and
   [`test-kit/tests/spl_token_swap.rs`](test-kit/tests/spl_token_swap.rs) for a complete
   worked example.

## Claude Code skill

[`skills/dex-integration`](skills/dex-integration) is a Claude Code skill that walks an
agent through the whole integration — the `Amm` contract method by method, the parity
testing workflow, and the handover checklist. Install it in the SDK repo you're working in:

```
/plugin marketplace add jup-ag/jupiter-amm-interface
/plugin install jupiter-amm
```

Then `/dex-integration`, or just describe the integration task and it activates on its own.

Without Claude Code, [`skills/dex-integration/SKILL.md`](skills/dex-integration/SKILL.md)
and its `references/` read fine as plain integration docs.

Questions? Ask in the [Jupiter Discord](https://discord.gg/jup) `#developer-support` channel.
