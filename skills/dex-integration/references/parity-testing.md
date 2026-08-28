# Proving parity with `jupiter-amm-test-kit`

The kit answers one question per swap: **does `quote.out_amount` equal the number of tokens
the program actually moves?**

For each registered swap it:

1. loads the pool's account snapshot from committed fixtures;
2. builds your `Amm` via `from_keyed_account` and drives `update()` from the snapshot;
3. spins up a fresh LiteSVM with the program `.so` and those accounts, clock pinned to the
   snapshot moment;
4. creates the user's ATAs, funding the source with *exactly* the swap input;
5. calls your `quote()`;
6. calls your `get_swap_and_account_metas()`, encodes the instruction data with your
   `encoder`, sends the transaction;
7. asserts the destination ATA's balance delta `==` `quote.out_amount`.

Because the source account holds exactly `in_amount`, a program that pulls more than it
should fails the transaction rather than quietly draining a padded balance.

## Test file shape

```rust
use jupiter_amm_interface::{Swap, solana::pubkey::{Pubkey, pubkey}};
use jupiter_amm_test_kit::{PoolTest, assert_pool_parity};

const POOL: Pubkey = pubkey!("...");
const MINT_A: Pubkey = pubkey!("...");
const MINT_B: Pubkey = pubkey!("...");

#[test]
fn pool_a_b() {
    let test = PoolTest::new(POOL)
        .add_swap(MINT_A, MINT_B, 1_000_000)      // small
        .add_swap(MINT_A, MINT_B, 100_000_000_000) // large
        .add_swap(MINT_B, MINT_A, 1_000_000);      // reverse

    assert_pool_parity::<MyAmm>(&test, encode_swap);
}

/// Native swap instruction data: discriminator + args, from the decision bits your
/// `get_swap_and_account_metas` put in `Swap::Placeholder { data }` plus the input amount.
fn encode_swap(swap: &Swap, in_amount: u64) -> Vec<u8> {
    match swap {
        Swap::Placeholder { data } => {
            let a_to_b = data.as_deref().map(|d| d[0] == 1).unwrap_or(true);
            let mut out = Vec::new();
            out.extend_from_slice(&DISCRIMINATOR);
            out.extend_from_slice(&in_amount.to_le_bytes());
            out.extend_from_slice(&0u64.to_le_bytes()); // min_amount_out: leave at 0
            out.push(a_to_b as u8);
            out
        }
        other => panic!("unexpected swap variant: {other:?}"),
    }
}
```

Each `add_swap` runs against a **fresh SVM** seeded from the same snapshot, so swaps don't
compound and order doesn't matter.

Set `minimum_amount_out` (or the program's equivalent slippage bound) to `0`. Asserting the
exact output is the kit's job; a bound here just converts a mismatch into an opaque program
error.

`PoolTest::params` passes AMM-specific JSON through to `from_keyed_account` — most AMMs
leave it `None`.

## Fixtures

Fixed convention, all paths relative to your crate root (where cargo runs tests):

| What | Path |
|---|---|
| Account snapshot | `tests/fixtures/accounts/<pool_pubkey>/<account_pubkey>.bin` |
| Your program | `tests/fixtures/<label>.so` — `Amm::label()` lowercased, spaces → underscores |
| A CPI dependency | `tests/fixtures/<name>.so` — `name` from `Amm::program_dependencies()` |

So `label() == "My DEX"` loads `tests/fixtures/my_dex.so`.

### Snapshotting from RPC

With the default `rpc` feature, the kit generates fixtures itself. When a test's fixture
directory is missing:

```sh
RPC=<mainnet-url> cargo test --test my_amm
```

It fetches the pool, builds the AMM, discovers every account the AMM needs
(`get_accounts_to_update`, reserve mints, the swaps' account metas, plus sysvars), pulls
them in one `getMultipleAccounts`, and writes the `.bin` layout above. Missing program
`.so` files are dumped from chain too; **existing ones are kept**, so a locally built,
not-yet-deployed program is never overwritten.

Refresh with `REFRESH=1 RPC=<url>`, or by deleting the pool's fixture directory.

Then **commit the fixtures** — the suite must pass offline in CI with no `RPC` set. That
also makes it a regression test: the same accounts, forever.

To test a different snapshot of the same pool (mid-tick-crossing, a stale oracle, a
paused state), point `test.fixtures_dir` at a second directory.

Other API, for custom flows: `test.regenerate_fixtures::<A>(url)`, `PoolSnapshot`
(`from_accounts` / `load_dir` / `save_dir` / `clock()` / `from_rpc`), and
`fetch_account(url, &pubkey)`.

## What to cover

Rounding bugs live in the extremes and in state transitions, not in the middle. Aim for:

- **Both directions** of every pool.
- **Amount extremes** — small enough that a fee truncates to zero (or gets bumped to one),
  large enough to move deep into the curve or hit a reserve/liquidity limit.
- **Every curve, fee tier, and pool variant** the program supports — one `PoolTest` each.
- **State transitions** — crossing a tick array or bin boundary, dynamic-fee windows,
  oracle freshness, near-empty pools.
- **Rejections** — assert that a paused or unsupported pool errors from
  `from_keyed_account`/`quote` rather than quoting nonsense. Plain `#[test]`s, no kit needed.

## Diagnosing failures

**`quote.out_amount does not match the on-chain output delta`** — the assert fired. The
transaction ran; the math disagrees.

- Off by a few units → rounding direction, or fees subtracted at a different step. Re-derive
  the program's math statement by statement; don't simplify.
- Off proportionally, or scaling with amount → wrong reserves/liquidity, wrong
  direction mapping, or missing state. Check `get_accounts_to_update()` lists everything
  and `update()` actually reads it.
- Correct one direction, wrong the other → direction mapping in `quote()` or the encoder's
  direction flag.

**`native swap transaction succeeds` panics** — the transaction failed; your quote wasn't
even reached in earnest. This is an instruction-construction problem, not math:

- account metas in the wrong order, or wrong `is_signer` / `is_writable` flags;
- an adapter-shaped meta list (e.g. a leading program-id account) instead of the native
  instruction's accounts;
- instruction data misencoded — wrong discriminator, wrong argument order or widths;
- a slippage bound left non-zero;
- a CPI target not declared in `program_dependencies()`, so its `.so` was never loaded.

**`read program ...so` / `pool ... not in snapshot`** — fixtures missing or misnamed. Check
the label→filename mapping and rerun with `RPC=` set.

**`from_keyed_account` panics** — the snapshot's pool account isn't what you expect; verify
the pool address and that your validation matches the on-chain layout.

## Limits of the kit

It covers the common case. Today that means:

- **ExactIn only.** Every swap is quoted and executed as `SwapMode::ExactIn`.
- **One native instruction per swap.** The encoder can dispatch between the program's swap
  instructions, but a flow needing orchestration *around* the instruction — SOL wrapping,
  ephemeral accounts, multi-instruction sequences — can't be reproduced.
- **Top-level invocation.** Programs that introspect the instructions sysvar and only
  accept a CPI from an approved caller can't be driven this way yet.
- **Account metas verbatim.** They must be exactly the native instruction's accounts, in
  on-chain order.
- **One signer** — the user keypair.
- **Plain token accounts.** Freshly created ATAs; token-2022 mints work, but transfer logic
  needing extra per-account state (transfer-hook side accounts) may not.
- **Defaults for the rest.** `FeeMode::Normal`, no referrer, clock pinned to the snapshot.

If one of these blocks the integration, raise it in the
[Jupiter Discord](https://discord.gg/jup) `#developer-support` channel or as an
[issue](https://github.com/jup-ag/jupiter-amm-interface/issues) instead of working around it
silently — the limits are what integrators have needed so far, not design walls.

## Dependency note

The kit pulls in LiteSVM, which pins the tree to one Solana crate generation. Exact versions
are held by the committed `Cargo.lock`. LiteSVM `0.14` is built against `wincode 0.5.5`,
while `solana-clock` / `solana-last-restart-slot` / `solana-slot-history` `3.2.0` moved to
`wincode 0.6.0` and break LiteSVM's sysvar (de)serialization — so **don't `cargo update`
those past `3.2.0`** without also moving LiteSVM to a wincode-0.6 release.
