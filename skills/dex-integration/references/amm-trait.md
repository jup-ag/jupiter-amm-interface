# The `Amm` trait, method by method

Reference for `jupiter_amm_interface::Amm`. Required methods first, then the defaults —
several of the defaults are effectively required for a good integration.

## Lifecycle

```
from_keyed_account(pool account)          once per pool
  → get_accounts_to_update()  ──┐
  → update(provider)          ──┘         repeatedly, as chain state advances
  → quote(params)                         very many times, between updates
  → get_swap_and_account_metas(params)    once a route is chosen
```

`update()` and `quote()` interleave for the lifetime of the process. Assume `quote()` is
called far more often than `update()`, and that both are called concurrently across many
pools.

---

## `from_keyed_account(&KeyedAccount, &AmmContext) -> Result<Self, AmmError>`

Build the AMM from its pool/market account.

```rust
pub struct KeyedAccount {
    pub key: Pubkey,                     // the pool address
    pub account: Account,                // its account, at some slot
    pub params: Option<serde_json::Value>, // extra config, AMM-defined
}
```

- **Validate.** Check `account.owner() == &PROGRAM_ID` and the discriminator / account
  variant. Return `AmmError` for anything you don't handle — a wrong curve type, an
  unsupported version, a foreign account. Do not guess.
- **Derive what is fixed for the pool's life**: PDAs (authority, vaults, oracles), mints,
  fee accounts, fee parameters, decimals.
- **Leave mutable state zeroed.** Reserves, tick arrays, oracle prices are filled by the
  first `update()`. Never quote before `update()` has run.
- `params` is optional AMM-specific JSON. Most AMMs use `None`. Use it only for
  configuration that cannot be read from chain.
- `AmmContext { clock_ref: ClockRef }` is a shared, live clock — `Arc`-backed atomics that
  the router keeps current. Clone it into `self` if your math is time-dependent (dynamic
  fees, oracle staleness, decay, epoch logic) and read it in `quote()`. Do not snapshot
  the numbers at construction; read through the `ClockRef` each time.

## `label(&self) -> AmmLabel` / `program_id(&self)` / `key(&self)`

- `label()` is a `&'static str` human name for the DEX, e.g. `"SPL Token Swap"`. It also
  determines the program fixture filename in the test kit (lowercased, spaces →
  underscores: `spl_token_swap.so`). Keep it stable — it appears in routing output.
- `program_id()` is the program that owns the accounts and executes the swap.
- `key()` is the pool/market address — the unique identity of this `Amm` instance.

For a DEX with a single program, get all three from the `single_program_amm!` macro:

```rust
single_program_amm!(MyAmm, MY_PROGRAM_ID, "My DEX");
// then
fn label(&self) -> AmmLabel { Self::LABEL }
fn program_id(&self) -> Pubkey { Self::PROGRAM_ID }
```

For several program deployments under one label, implement `AmmProgramIdToLabel` with the
full `PROGRAM_ID_TO_LABELS` table instead.

## `get_reserve_mints(&self) -> Vec<Pubkey>`

The mints this pool can trade. **Order is meaningful**: with `unidirectional() == true`,
trading goes from the first mint to the second only. Keep the order stable across calls.

If you cannot know the mints until state is loaded, override
`requires_update_for_reserve_mints()` to return `true`.

## `get_accounts_to_update(&self) -> Vec<Pubkey>`

Every account `update()` needs in order to make `quote()` correct. Typically: vaults or
reserves, tick arrays / bin arrays, oracles, dynamic fee/config accounts.

- **Completeness is correctness.** An account you forget is an account whose stale value
  you quote against.
- **Keep it tight.** Every entry costs the router account-fetch bandwidth for every pool of
  your DEX. Don't list the pool's static config accounts you already decoded in
  `from_keyed_account`, and don't list accounts only the *swap instruction* needs (those
  belong in `get_swap_and_account_metas`).
- If the list can change between calls — tick arrays that follow the active price, bins
  that follow liquidity — you **must** override `has_dynamic_accounts()` to `true`.

## `update(&mut self, impl AccountProvider) -> Result<(), AmmError>`

Refresh mutable state from the accounts you asked for.

```rust
fn update(&mut self, account_provider: impl AccountProvider) -> Result<(), AmmError> {
    let vault = account_provider.try_get(&self.vault_a)?;   // Err if absent
    // let vault = account_provider.get(&self.vault_a);     // Option, if optional
    self.reserve_a = decode_amount(vault.data())?;
    Ok(())
}
```

- `AccountProvider::try_get` returns `Err(AccountNotFoundError)` — which converts into
  `AmmError` via `?`. Use `get` only for genuinely optional accounts.
- **This is where all expensive work belongs**: deserialization, decompression, building
  tick/bin lookup structures, precomputing anything `quote()` would otherwise recompute per
  call.
- Accounts may be absent or malformed. Return `AmmError`; never panic, never index a slice
  without a length check.
- **No I/O.** `update()` does not fetch anything. Accounts are handed to it. Making an RPC
  call here is the single worst thing an integration can do to router latency.
- Leave `self` usable on error. Prefer computing into locals and assigning at the end over
  half-mutating and returning `Err`.
- Large precomputed structures should live behind `Arc` — `Amm: Clone` and the router
  clones freely.

## `quote(&self, &QuoteParams) -> Result<Quote, AmmError>`

The hot path, and the thing that has to be exactly right.

```rust
pub struct QuoteParams {
    pub amount: u64,          // in_amount for ExactIn, out_amount for ExactOut
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub swap_mode: SwapMode,  // ExactIn | ExactOut
    pub fee_mode: FeeMode,    // Normal | Ultra
}

pub struct Quote {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_mint: Pubkey,
    pub fee_pct: Decimal,
}
```

Rules:

- **Exact, not approximate.** `out_amount` must equal what the program transfers, to the
  unit. Reproduce the on-chain arithmetic literally: same integer widths (`u128` if the
  program uses `u128`), same order of operations, same rounding direction on every
  division, fees subtracted at the same step. Algebraically-equivalent simplifications
  drift by a unit; a unit is a failed integration.
- **Pure.** No I/O, no RPC, no locks, no mutation, no global state, no randomness, no
  wall-clock reads (use the `ClockRef` from `AmmContext`). Given the same state and params,
  the same answer.
- **Cheap.** Avoid allocation and avoid recomputing what `update()` could have cached.
- **Fail loudly rather than wrongly.** Insufficient liquidity, an unsupported direction, a
  stale oracle, arithmetic that would overflow: return `AmmError`. Never clamp, saturate,
  or return a best guess — a wrong quote becomes a failed or badly-priced user swap.
- **`swap_mode`.** If `supports_exact_out()` is `false` (the default), you may reject
  `ExactOut`. If it is `true`, `amount` is the desired output and `in_amount` is what you
  compute.
- **`fee_mode`** distinguishes normal routing from Ultra. Unless the program charges
  differently in that case, it does not affect the math — but if it does, the quote must
  reflect it.
- Fill `fee_amount` / `fee_mint` / `fee_pct` where the program has a real fee. `fee_pct` is
  a `rust_decimal::Decimal` (re-exported at `jupiter_amm_interface::rust_decimal`).

## `get_swap_and_account_metas(&self, &SwapParams) -> Result<SwapAndAccountMetas, AmmError>`

Describe the swap instruction: which variant, and its accounts.

```rust
pub struct SwapAndAccountMetas {
    pub swap: Swap,
    pub account_metas: Vec<AccountMeta>,
}
```

- **Use `Swap::Placeholder { data }` until Jupiter integrates the DEX** — a new AMM has no
  variant in the `Swap` enum yet. `data: Option<Vec<u8>>` carries the *minimal decision
  bits* the instruction encoder needs: a direction flag, an operation tag when the program
  has several swap instructions. Do not stuff a fully-encoded instruction in there.
- **`account_metas` must be the native instruction's accounts, in on-chain order**, with
  correct `is_signer` / `is_writable` flags. No leading program-id account, no
  adapter-shaped wrapper.
- Useful `SwapParams` fields:
  - `source_token_account` / `destination_token_account` — the user's token accounts.
  - `token_transfer_authority` — signs the transfer; may be the user or a program
    authority. Mark it `is_signer: true`.
  - `user` — the actual swapping user; `payer` — pays rent for accounts the swap creates.
  - `source_mint` / `destination_mint` — direction. `in_amount` / `out_amount` /
    `swap_mode`.
  - `quote_mint_to_referrer` — referrer token accounts by mint, if the program supports
    referral fees.
  - `jupiter_program_id`, and `placeholder_account_meta()` which returns a read-only meta on
    it — use it for optional-account slots and as a terminator when the program consumes
    remaining accounts.
  - `missing_dynamic_accounts_as_default` — when `true`, substitute `Pubkey::default()` for
    dynamic accounts you cannot resolve instead of returning `Err`. Used for crawling
    markets whose tick arrays don't exist yet.
- Derive addresses deterministically. Return `AmmError` if a required account can't be
  resolved (unless `missing_dynamic_accounts_as_default`).

---

## Defaults worth overriding

All of these have defaults, but the defaults are only right for the simplest AMMs.

| Method | Default | Override when |
|---|---|---|
| `has_dynamic_accounts()` | `false` | `get_accounts_to_update()` can return a different set between calls (tick arrays, bins, price-dependent accounts). **Getting this wrong means quoting against accounts that were never fetched.** |
| `requires_update_for_reserve_mints()` | `false` | The tradable mints aren't known until after `update()`. |
| `supports_exact_out()` | `false` | `quote()` genuinely handles `SwapMode::ExactOut`. Leave `false` rather than approximating it. |
| `unidirectional()` | `false` | The pool only trades first→second reserve mint (see `get_reserve_mints` ordering). |
| `get_accounts_len()` | `32` | **Almost always.** Return the real length of `account_metas`. The default assumes a near-full legacy transaction and penalizes the pool in route selection. |
| `underlying_liquidities()` | `None` | The pool's liquidity is shared with another market Jupiter also routes (e.g. an order book the AMM sits on). Return the shared identifier so Jupiter won't route through the same liquidity twice in one transaction. |
| `is_active()` | `true` | Return `false` for paused, uninitialized, empty, or otherwise unswappable pools — cheaper than making every `quote()` fail. |
| `program_dependencies()` | `vec![]` | The swap CPIs into another program. Returns `(Pubkey, String)` pairs; the `String` names the `.so` the test kit loads. |

---

## Error handling

```rust
pub enum AmmError {
    AccountNotFound(AccountNotFoundError),  // from AccountProvider::try_get via `?`
    Io(io::Error),
    Json(serde_json::Error),
    Program(ProgramError),
    Custom(String),
}
```

`AmmError` has `From<&str>` and `From<String>`, so `AmmError::from("pool is paused")` and
`return Err("bad curve".into())` both work. Prefer messages that name the pool condition —
they surface in routing diagnostics.

Nothing in an `Amm` impl may panic. `unwrap`, `expect`, `slice[i]`, `try_into().unwrap()`,
and unchecked arithmetic are all reachable from account bytes you don't control. Use
`checked_*` arithmetic, length-check before slicing, and convert failures into `AmmError`.
