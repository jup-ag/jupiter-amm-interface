use std::path::PathBuf;

use jupiter_amm_interface::{
    Amm, AmmContext, ClockRef, KeyedAccount, QuoteParams, Swap, SwapAndAccountMetas, SwapMode,
    SwapParams,
};
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::Instruction;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::snapshot::{PoolSnapshot, update_amm_from_snapshot};
use crate::svm::{create_funded_ata, load_programs, pool_fixtures_dir, seed_svm, token_amount};

const JUPITER_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");

/// A single swap to assert: `amount` of `input_mint` into `output_mint`.
#[derive(Clone, Copy)]
pub struct SwapTest {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub amount: u64,
}

/// A pool to verify: where its snapshot fixtures live and the swaps to assert on them.
pub struct PoolTest {
    /// The pool/market account key (built into the AMM via `from_keyed_account`).
    pub pool: Pubkey,
    /// Directory of `<pubkey>.bin` fixtures (see [`PoolSnapshot`]).
    /// Defaults to `tests/fixtures/accounts/<pool>`.
    pub fixtures_dir: PathBuf,
    /// Swaps to assert against the snapshot state.
    pub swaps: Vec<SwapTest>,
    /// Optional `params` passed to `from_keyed_account` (most AMMs: `None`).
    pub params: Option<serde_json::Value>,
}

impl PoolTest {
    pub fn new(pool: Pubkey) -> Self {
        Self {
            pool,
            fixtures_dir: pool_fixtures_dir(&pool),
            swaps: Vec::new(),
            params: None,
        }
    }

    /// Add a swap to assert. Each swap runs in its own fresh SVM seeded from the snapshot.
    pub fn add_swap(mut self, input_mint: Pubkey, output_mint: Pubkey, amount: u64) -> Self {
        self.swaps.push(SwapTest {
            input_mint,
            output_mint,
            amount,
        });
        self
    }
}

/// Build the Amm from the test's snapshot and assert every swap's quote matches the on-chain
/// swap.
///
/// `encoder` is needed to build the program's actual swap instruction data (discriminator + args)
/// as it is not part of the `Amm` trait. It is called once per swap with
/// the `Swap::Placeholder{ data }` the `Amm` returned for that swap, and the input amount.
/// Keep the variant minimal (an operation tag, a direction flag)
/// and build the instruction data here, from those decision bits plus the amount.
///
/// Programs are loaded from `tests/fixtures/<name>.so`.
/// For the amm, `<name>` is `Amm::label()` in snakecase (e.g. "SPL Token Swap" -> `spl_token_swap.so`).
/// For `Amm::program_dependencies()`, the String is used as-is.
pub fn assert_pool_parity<A: Amm>(test: &PoolTest, encoder: impl Fn(&Swap, u64) -> Vec<u8>) {
    assert!(!test.swaps.is_empty(), "PoolTest has no swaps to assert");

    // Missing fixtures are JIT snapshotted when `RPC` is set.
    // Set `REFRESH=1` (with `RPC`) to re-snapshot existing ones.
    #[cfg(feature = "rpc")]
    if std::env::var_os("REFRESH").is_some() || !test.fixtures_dir.exists() {
        let url = std::env::var("RPC").unwrap_or_else(|_| {
            panic!(
                "fixtures {:?} missing or REFRESH set — rerun with RPC=<url> to snapshot them",
                test.fixtures_dir
            )
        });
        test.regenerate_fixtures::<A>(&url)
            .unwrap_or_else(|e| panic!("snapshot fixtures: {e}"));
    }

    let snapshot = PoolSnapshot::load_dir(&test.fixtures_dir)
        .unwrap_or_else(|e| panic!("load fixtures {:?}: {e}", test.fixtures_dir));

    let amm_context = AmmContext {
        clock_ref: ClockRef::from(snapshot.clock().unwrap_or_default()),
    };
    let keyed_account = KeyedAccount {
        key: test.pool,
        account: snapshot
            .get(&test.pool)
            .unwrap_or_else(|| panic!("pool {} not in snapshot", test.pool))
            .clone(),
        params: test.params.clone(),
    };
    let mut amm = A::from_keyed_account(&keyed_account, &amm_context)
        .unwrap_or_else(|e| panic!("from_keyed_account: {e}"));
    update_amm_from_snapshot(&mut amm, &snapshot);
    let programs = load_programs(&amm);

    for swap in &test.swaps {
        assert_swap_parity(&amm, &snapshot, &programs, swap, &encoder);
    }
}

/// Run one swap against a fresh SVM built from the snapshot and assert quote == on-chain.
fn assert_swap_parity<A: Amm>(
    amm: &A,
    snapshot: &PoolSnapshot,
    programs: &[(Pubkey, Vec<u8>)],
    swap_test: &SwapTest,
    encoder: &impl Fn(&Swap, u64) -> Vec<u8>,
) {
    let mut svm = seed_svm(snapshot, programs);

    let user = Keypair::new();
    let user_pk = user.pubkey();
    svm.airdrop(&user_pk, 100 * 1_000_000_000)
        .expect("airdrop user");
    // The user swaps between their ATAs. The source is seeded with exactly the swap's
    // input; a program that pulls more than `in_amount` fails the transaction instead of
    // draining a padded balance.
    let source_token_account = create_funded_ata(
        &mut svm,
        snapshot,
        &swap_test.input_mint,
        &user,
        swap_test.amount,
    );
    let destination_token_account =
        create_funded_ata(&mut svm, snapshot, &swap_test.output_mint, &user, 0);

    let quote = amm
        .quote(&QuoteParams {
            amount: swap_test.amount,
            input_mint: swap_test.input_mint,
            output_mint: swap_test.output_mint,
            swap_mode: SwapMode::ExactIn,
            fee_mode: Default::default(),
        })
        .expect("quote");

    let SwapAndAccountMetas {
        swap,
        account_metas,
    } = amm
        .get_swap_and_account_metas(&SwapParams {
            swap_mode: SwapMode::ExactIn,
            in_amount: swap_test.amount,
            out_amount: swap_test.amount,
            source_mint: swap_test.input_mint,
            destination_mint: swap_test.output_mint,
            source_token_account,
            destination_token_account,
            token_transfer_authority: user_pk,
            user: user_pk,
            payer: user_pk,
            quote_mint_to_referrer: None,
            jupiter_program_id: &JUPITER_PROGRAM_ID,
            missing_dynamic_accounts_as_default: false,
        })
        .expect("get_swap_and_account_metas");

    let swap_ix = Instruction {
        program_id: amm.program_id(),
        accounts: account_metas,
        data: encoder(&swap, swap_test.amount),
    };

    let out_before = token_amount(&svm, &destination_token_account);

    let ixs = [
        ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
        swap_ix,
    ];
    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&ixs, Some(&user_pk), &[&user], blockhash);
    svm.send_transaction(tx)
        .expect("native swap transaction succeeds");

    let out_delta = token_amount(&svm, &destination_token_account) - out_before;

    assert_eq!(
        quote.out_amount, out_delta,
        "quote.out_amount does not match the on-chain output delta",
    );
}
