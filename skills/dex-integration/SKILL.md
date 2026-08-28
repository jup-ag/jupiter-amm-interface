---
name: dex-integration
description: Integrate a Solana DEX/AMM into Jupiter routing by implementing the `jupiter-amm-interface` `Amm` trait, then proving the quote matches the on-chain program with `jupiter-amm-test-kit`. Use when building or reviewing a Jupiter DEX integration SDK — writing `from_keyed_account`/`update`/`quote`/`get_swap_and_account_metas`, choosing accounts to update, wiring `Swap::Placeholder`, setting up parity tests and fixtures, or debugging a quote that disagrees with on-chain output, a failing parity transaction, or clashing `solana-*` crate versions.
---

# Integrating a DEX into Jupiter

Jupiter routes through a DEX via one Rust trait: `Amm`, from
[`jupiter-amm-interface`](https://github.com/jup-ag/jupiter-amm-interface). You ship a crate
that implements it; Jupiter loads pools, calls `update()` as chain state changes, calls
`quote()` a very large number of times to search routes, and calls
`get_swap_and_account_metas()` to build the transaction.

Two properties decide whether the integration works:

1. **`quote()` is exact.** It must return the same number of output tokens the program
   actually produces — to the unit, not to a tolerance.
2. **`quote()` is pure and fast.** No I/O, no locks, no allocation-heavy work. It is on the
   router's hot path.

Everything below serves those two. Work through the phases in order.

## Phase 1 — Read the program first

Do not start writing Rust. Collect these facts and write them down; each one maps to a
piece of the impl. If the answer isn't in the program source, find it in the IDL or by
decoding a real pool account and a real swap transaction.

- **Program ID**, and any program it CPIs into (token program, oracle, vault).
- **Pool/market account**: layout, discriminator, how to reject a foreign or wrong-variant
  account.
- **Which accounts the swap math reads.** Vault balances? Tick arrays / bins? An oracle? A
  global config? Do they change per swap direction or per price? This becomes
  `get_accounts_to_update()`, and if the set varies, `has_dynamic_accounts()`.
- **The exact swap math**, as integer operations: order of operations, rounding direction at
  every division, where fees are subtracted, what saturates vs what errors.
- **The native swap instruction**: discriminator, argument encoding, and the full account
  list *in order*, with signer/writable flags.
- **Direction and variants**: one instruction for both directions, or separate
  buy/sell/deposit/redeem instructions?

A concrete, complete worked example of all of this — a real `Amm` plus its parity test —
lives in the interface repo at
[`test-kit/tests/spl_token_swap.rs`](https://github.com/jup-ag/jupiter-amm-interface/blob/main/test-kit/tests/spl_token_swap.rs).
Read it before writing your own; it is ~250 lines and covers the whole shape.

## Phase 2 — Set up the crate

```toml
[dependencies]
jupiter-amm-interface = "0.6"

[dev-dependencies]
jupiter-amm-test-kit = "0.1"
```

**Do not add `solana-*` dependencies.** Take every Solana type from
`jupiter_amm_interface::solana`:

```rust
use jupiter_amm_interface::solana::{
    account::ReadableAccount,
    clock::Clock,
    instruction::AccountMeta,
    program_error::ProgramError,
    pubkey::{Pubkey, pubkey},
};
```

`rust_decimal` and `serde_json` are re-exported at the crate root for the same reason.

This is not a style preference. A second Solana crate generation in the tree produces two
incompatible `Pubkey` types and a wall of `expected Pubkey, found Pubkey` errors — the
single most common thing that stalls an integration. If you need a Solana crate that isn't
re-exported, ask for it to be added rather than declaring your own.

Same rule for your own vendored math: if the DEX's existing on-chain crate is pinned to an
older Solana generation, copy the math into this crate rather than depending on it.

## Phase 3 — Implement `Amm`

Write the methods in this order — each one's output feeds the next:

`from_keyed_account` → `label` / `program_id` / `key` → `get_reserve_mints` →
`get_accounts_to_update` → `update` → `quote` → `get_swap_and_account_metas` → the
optional-but-important defaults.

**Read [`references/amm-trait.md`](references/amm-trait.md) now.** It is the contract for
every method: what it may and may not do, which defaults must be overridden and when, and
the pitfall attached to each. Do not implement from the trait's doc comments alone — they
are one line each and leave the sharp parts out.

The three rules that cause the most rework, stated up front:

- **Nothing panics.** `unwrap`, `expect`, slice indexing, and arithmetic overflow on
  account data are all reachable from untrusted chain state. Return `AmmError`. A panic in
  `quote` or `update` is a fault in the router process, not a bad quote.
- **All heavy work goes in `update()`.** Deserialize, precompute, build lookup tables
  there. `quote()` reads already-prepared state and does arithmetic.
- **`Amm: Clone` and clones are frequent.** Put large immutable precomputed tables behind
  `Arc` so cloning stays cheap.

Reproduce the program's arithmetic *literally* — same integer widths, same order of
operations, same rounding direction, fees subtracted at the same step. Algebraically
equivalent but "cleaner" math is off by one unit, and one unit is a failed integration.

## Phase 4 — Prove parity against the real program

An integration is not done because it compiles and returns plausible numbers. Prove it:
`jupiter-amm-test-kit` snapshots a live pool, runs your `quote()`, executes the program's
real swap instruction in LiteSVM against the same accounts, and asserts the on-chain output
delta equals `quote.out_amount` exactly.

```rust
#[test]
fn my_pool() {
    let test = PoolTest::new(POOL)
        .add_swap(INPUT_MINT, OUTPUT_MINT, 100_000_000)
        .add_swap(OUTPUT_MINT, INPUT_MINT, 50_000_000);

    assert_pool_parity::<MyAmm>(&test, encode_swap);
}
```

The only extra code is `encode_swap`, which builds the native instruction data from the
`Swap::Placeholder { data }` your `get_swap_and_account_metas` returned.

**Read [`references/parity-testing.md`](references/parity-testing.md)** for the fixture
convention, snapshotting from RPC, the encoder contract, and what the kit does *not* cover.

Cover more than the happy path. One pool with one swap size proves almost nothing —
rounding bugs hide in the extremes:

- both directions;
- tiny amounts (where a fee truncates to 0 or gets bumped to 1) and large amounts (deep
  into the curve, near reserve/liquidity limits);
- each pool *type* and each curve/fee variant the program supports;
- each distinct state shape: crossing a tick array or bin boundary, a stale or fresh
  oracle, dynamic-fee windows, a paused or empty pool.

Add a `PoolTest` per pool and per interesting snapshotted state, and commit the fixtures so
the suite runs offline in CI.

## Phase 5 — Before handing the SDK over

Check each of these; they are what gets reviewed.

- [ ] `cargo test` passes offline, from committed fixtures, with no `RPC` set.
- [ ] Parity tests cover every pool type, both directions, and small/large amounts.
- [ ] No `solana-*` dependency in `Cargo.toml`; one Solana generation in `Cargo.lock`.
- [ ] No `unwrap`/`expect`/`panic!`/indexing reachable from account data in `update`,
      `quote`, or `get_swap_and_account_metas`.
- [ ] `quote()` does no I/O and takes no locks; heavy work is in `update()`.
- [ ] `get_accounts_to_update()` returns everything `quote()` depends on — and
      `has_dynamic_accounts()` is `true` if that list can change.
- [ ] `get_accounts_len()` reports the real account count (the default `32` penalizes you
      in routing).
- [ ] `underlying_liquidities()` returns the shared source if the pool trades on liquidity
      another market also exposes.
- [ ] `is_active()` returns `false` for paused/uninitialized/empty pools instead of
      quoting them.
- [ ] `supports_exact_out()` is only `true` if `quote()` genuinely handles
      `SwapMode::ExactOut`.
- [ ] `get_swap_and_account_metas()` returns the native instruction's accounts in exact
      on-chain order with correct signer/writable flags.
- [ ] A short README: program ID, which pools/curves are supported, how to run the tests,
      how to refresh fixtures.

## Debugging

| Symptom | Where to look |
|---|---|
| `expected Pubkey, found Pubkey`, duplicate `solana-*` in `Cargo.lock` | A `solana-*` dep of your own, or a vendored crate on an older generation — Phase 2 |
| Parity test: quote off by a few units | Rounding direction or fee-subtraction order differs from the program; re-derive the math statement by statement |
| Parity test: quote off by a lot, or scales with amount | Wrong reserves/liquidity, wrong direction mapping, or stale state — check `get_accounts_to_update` and `update` |
| Parity test: transaction fails, not the assert | Account metas wrong (order, flags, or adapter-shaped), or instruction data misencoded — `references/parity-testing.md` |
| Quote correct at rest, wrong under load | `quote()` reading something `update()` didn't refresh, or state mutated outside `update()` |
| Quote correct, route never picked | `is_active()`, `get_accounts_len()`, or `get_reserve_mints()` ordering with `unidirectional()` |

## Getting help

Ask in the [Jupiter Discord](https://discord.gg/jup) `#developer-support` channel, or open
an issue on
[jup-ag/jupiter-amm-interface](https://github.com/jup-ag/jupiter-amm-interface/issues).
If a test-kit limitation blocks you, say so there rather than working around it silently —
the limits reflect what integrators have needed so far, not design walls.
