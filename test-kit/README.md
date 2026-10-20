# jupiter-amm-test-kit

A self-verification harness for Jupiter AMM integrations. It proves that an SDK's
`Amm::quote()` matches what the on-chain program actually does.

## What it does

1. Load a `PoolSnapshot` (accounts dumped from a live pool).
2. Run the SDK's pure `Amm::quote`.
3. Execute the AMM program's swap instruction in LiteSVM.
4. Assert the realized on-chain token delta equals the quote exactly.

Besides the `Amm` impl itself, the only integrator-written piece is a small `encoder` closure that
encodes the program's native swap instruction data (discriminator + args).

A new AMM won't have its own `Swap` variant until Jupiter integrates it, so it should use
`Swap::Placeholder { data }` in the meantime. `data` is how the `Amm` passes information
to the encoder — a direction flag, or an operation tag when the AMM has several swap
instructions the encoder must dispatch between.

## Writing a test suite

Register one `PoolTest` per pool (or per snapshotted state) and assert it — each becomes a
`#[test]`. This is exactly what [`tests/spl_token_swap.rs`](tests/spl_token_swap.rs) does;
copy it.

```rust
#[test]
fn my_pool() {
    let test = PoolTest::new(POOL)
        .add_swap(INPUT_MINT, OUTPUT_MINT, 100_000_000)   // add as many swaps as needed
        .add_swap(OUTPUT_MINT, INPUT_MINT, 50_000_000);   // each runs against a fresh SVM

    assert_pool_parity::<MyAmm>(&test, encode_swap);
}

fn encode_swap(swap: &Swap, in_amount: u64) -> Vec<u8> {
    // Build the program's native swap instruction data (discriminator + args) from the
    // `Swap::Placeholder { data }` decision bits and the input amount.
}
```

No paths, no program lists — everything follows a fixed fixtures convention:

- account snapshots live at `tests/fixtures/accounts/<pool>` (paths are relative to the
  SDK crate's root, where cargo runs its tests);
- the AMM's program loads from `tests/fixtures/<label>.so` (`Amm::label()`
  lowercased, spaces as underscores — label "SPL Token Swap" loads `spl_token_swap.so`),
  and anything it CPIs into comes from `Amm::program_dependencies()` as
  `(program_id, name)` pairs, loading `<name>.so`.

To test a **different pool**, add another `PoolTest`; for a different snapshot of the same
pool, override `test.fixtures_dir`.

Run the suite: `cargo test -p jupiter-amm-test-kit --test spl_token_swap`.

## Getting fixtures

With the `rpc` feature (on by default), the kit snapshots a live pool — no manual account
dumping. When a test's fixtures are missing, the parity test snapshots them itself:

```sh
RPC=<mainnet-url> cargo test --test my_amm
```

To refresh existing fixtures, add `REFRESH=1` (or delete the pool's
`tests/fixtures/accounts/<pool>` dir and rerun).

Under the hood the kit fetches the pool account, builds the AMM from it, discovers every
account it needs (`get_accounts_to_update`, reserve mints, the swaps' account metas, plus
sysvars), fetches them in one `getMultipleAccounts` batch, and writes `<pubkey>.bin`
files into the conventional layout above. Any *missing* program `.so` is dumped from
chain too; existing ones are kept, so a locally built (not yet deployed) program is never
overwritten. Commit all of it and the parity test runs fully offline.

## API

- `PoolTest::new(pool).add_swap(in, out, amount)…` — a registered pool.
- `assert_pool_parity::<A>(&test, encoder)` — build the AMM from the snapshot and assert
  every swap; call it from a `#[test]`.
- `test.regenerate_fixtures::<A>(url)` (feature `rpc`) — write the test's fixtures from a
  live RPC endpoint. `assert_pool_parity` calls it itself for missing fixtures when
  `RPC` is set; it's public for explicit refresh flows.
- `PoolSnapshot` — `from_accounts` / `load_dir` / `save_dir` / `clock()`, and (feature
  `rpc`) `from_rpc(url, &amm, &swaps)` for custom snapshot flows.
- `fetch_account(url, &pubkey)` (feature `rpc`) — fetch a single account.

## Limitations

The kit intentionally covers the common case first; today that means:

- **ExactIn only.** Every swap is quoted and executed as `SwapMode::ExactIn`. No
  ExactOut assertions yet.
- **One native instruction per swap.** The encoder can route different `Swap` variants
  to different instructions (buy vs sell, deposit vs withdraw), but a variant that needs
  orchestration *around* the instruction in production — SOL wrapping, ephemeral
  accounts, multi-instruction flows — can't be reproduced.
- **The swap must work as a plain top-level instruction.** Programs that introspect the
  instructions sysvar and reject anything but a CPI from an approved caller can't be
  driven this way yet.
- **Account metas are used verbatim.** `get_swap_and_account_metas` must return exactly
  the native instruction's accounts in on-chain order — adapter-shaped metas (e.g. a
  leading program-id account) will fail the transaction.
- **One signer.** The user keypair is the only signer; swaps requiring additional
  transaction signers aren't supported.
- **Plain token accounts.** The user's accounts are freshly created ATAs; token-2022
  mints work, but transfer logic that needs extra per-account state (e.g. transfer-hook
  side accounts) may not.
- **Defaults for the rest.** `FlowType::Binary`, no referrer, and the SVM clock pinned to the
  snapshot moment.

If one of these is a blocker — or the kit isn't flexible enough in some other way —
please [open an issue](https://github.com/jup-ag/jupiter-amm-interface/issues) or ask in
the [Jupiter Discord](https://discord.gg/jup) `#developer-support` channel rather than
working around it silently. These limits reflect what integrators have needed so far,
not hard design walls.

## Dependency notes

This crate pulls in LiteSVM, which fixes the dependency tree to a specific Solana crate
generation:

- The `jupiter-amm-interface` crate is on `solana-account = "4"` so its `Account` type
  unifies with LiteSVM's.
- Exact versions are held by the committed `Cargo.lock`; the manifest uses ordinary caret
  deps (no exact pins). This matters because LiteSVM `0.14` is built against `wincode 0.5.5`,
  but the `3.2.0` releases of `solana-clock` / `solana-last-restart-slot` /
  `solana-slot-history` bumped to `wincode 0.6.0`, which breaks LiteSVM's sysvar
  (de)serialization. The lockfile keeps those at `3.1.x`; **don't `cargo update` them past
  `3.2.0`** without also moving LiteSVM to a wincode-0.6 release.
