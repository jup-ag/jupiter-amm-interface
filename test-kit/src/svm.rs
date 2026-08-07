//! LiteSVM environment seeding: load the AMM's programs and snapshot accounts, create
//! user token accounts, and read token balances back out.

use std::path::PathBuf;

use jupiter_amm_interface::Amm;
use litesvm::LiteSVM;
use solana_account::ReadableAccount;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::snapshot::PoolSnapshot;

const ATA_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSTEM_PROGRAM_ID: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");
const NATIVE_MINT: Pubkey = solana_pubkey::pubkey!("So11111111111111111111111111111111111111112");

// The fixtures layout: account snapshots at `tests/fixtures/accounts/<pool>`, programs at
// `tests/fixtures/<name>.so`. Paths are relative to the crate under test — cargo runs
// test binaries from the package root.

pub(crate) fn pool_fixtures_dir(pool: &Pubkey) -> PathBuf {
    PathBuf::from("tests/fixtures/accounts").join(pool.to_string())
}

pub(crate) fn program_path(name: &str) -> PathBuf {
    PathBuf::from("tests/fixtures").join(format!("{name}.so"))
}

/// The `(program_id, name)` pairs to load for `amm`: its own program (named after its
/// label, lowercased with underscores) and its `program_dependencies()`.
pub(crate) fn program_names<A: Amm>(amm: &A) -> Vec<(Pubkey, String)> {
    let mut programs = vec![(
        amm.program_id(),
        amm.label().to_lowercase().replace(' ', "_"),
    )];
    programs.extend(amm.program_dependencies());
    programs
}

/// Read each of `amm`'s programs from its conventional `<name>.so` path.
pub(crate) fn load_programs<A: Amm>(amm: &A) -> Vec<(Pubkey, Vec<u8>)> {
    program_names(amm)
        .into_iter()
        .map(|(program_id, name)| {
            let path = program_path(&name);
            let bytes =
                std::fs::read(&path).unwrap_or_else(|e| panic!("read program {path:?}: {e}"));
            (program_id, bytes)
        })
        .collect()
}

/// A fresh SVM with the programs and the snapshot's accounts loaded. LiteSVM syncs its
/// sysvar cache — including the clock — from the snapshotted sysvar accounts.
pub(crate) fn seed_svm(snapshot: &PoolSnapshot, programs: &[(Pubkey, Vec<u8>)]) -> LiteSVM {
    let mut svm = LiteSVM::new();
    for (program_id, bytes) in programs {
        svm.add_program(*program_id, bytes)
            .expect("add AMM program");
    }
    for (pubkey, account) in &snapshot.accounts {
        if !account.executable {
            svm.set_account(*pubkey, account.clone())
                .expect("set snapshot account");
        }
    }
    svm
}

/// Create the wallet's associated token account for `mint` — through the ATA program, so
/// the account is exactly what a real user brings (correct address, token-2022 account
/// extensions initialized) — and seed it with `amount`.
///
/// The ATA derivation and instruction are built by hand: the published
/// `spl-associated-token-account-interface` is still on the `solana-pubkey` 3 generation,
/// which doesn't unify with this crate's types.
pub(crate) fn create_funded_ata(
    svm: &mut LiteSVM,
    snapshot: &PoolSnapshot,
    mint: &Pubkey,
    wallet: &Keypair,
    amount: u64,
) -> Pubkey {
    let token_program = snapshot
        .get(mint)
        .map(|a| *a.owner())
        .expect("mint present in snapshot");
    let wallet_pk = wallet.pubkey();
    let (ata, _) = Pubkey::find_program_address(
        &[wallet_pk.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ATA_PROGRAM_ID,
    );

    let create_ix = Instruction {
        program_id: ATA_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(wallet_pk, true), // funding payer
            AccountMeta::new(ata, false),
            AccountMeta::new_readonly(wallet_pk, false), // wallet
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![0], // Create
    };
    let blockhash = svm.latest_blockhash();
    let tx =
        Transaction::new_signed_with_payer(&[create_ix], Some(&wallet_pk), &[wallet], blockhash);
    svm.send_transaction(tx).expect("create ATA");

    if amount > 0 {
        let mut account = svm.get_account(&ata).expect("created ATA");
        // amount is at offset 64 (after mint[32] + owner[32]) for both token programs.
        account.data[64..72].copy_from_slice(&amount.to_le_bytes());
        if *mint == NATIVE_MINT {
            account.lamports += amount; // keep the native (wSOL) account backed
        }
        svm.set_account(ata, account).expect("fund ATA");
    }
    ata
}

pub(crate) fn token_amount(svm: &LiteSVM, token_account: &Pubkey) -> u64 {
    let account = svm.get_account(token_account).expect("token account");
    // amount is at offset 64 (after mint[32] + owner[32]).
    u64::from_le_bytes(account.data[64..72].try_into().unwrap())
}
