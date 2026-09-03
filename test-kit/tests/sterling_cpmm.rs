//! Parity test suite — Sterling Sovereign CPMM (92C / SLUSD, x·y=k, 0 bps).
//!
//! Implémente le trait `Amm` de `jupiter-amm-interface` pour la pool souveraine
//! `4wc5NC3ejYC5y4vQD4Eg3PsYW9ArZQV18fY1eHCJfxJQ` et prouve via LiteSVM que
//! `Amm::quote()` reproduit exactement le programme on-chain `7v9sLrk…`.
//!
//!   cargo test -p sterling-cpmm-testkit --test sterling_cpmm
//!
//! Fixtures manquantes ? Rerun avec un RPC mainnet (snapshot JIT) :
//!   RPC=<mainnet-url> cargo test --test sterling_cpmm
//!   REFRESH=1 RPC=<mainnet-url> cargo test --test sterling_cpmm   # re-snapshot

use jupiter_amm_interface::{
    AccountProvider, Amm, AmmContext, AmmError, AmmLabel, KeyedAccount, Quote, QuoteParams,
    SingleProgramAmm, Swap, SwapAndAccountMetas, SwapParams, single_program_amm,
};
use jupiter_amm_test_kit::{PoolTest, assert_pool_parity};
use solana_account::ReadableAccount;
use solana_instruction::AccountMeta;
use solana_pubkey::{Pubkey, pubkey};

// ── Identité on-chain (données publiques) ─────────────────────────────────
const STERLING_CPMM_PROGRAM: Pubkey =
    pubkey!("7v9sLrk92NNLLUfXLJw3o7MycZNvwsTK6kLWfWb8vcVA");
const STERLING_POOL: Pubkey = pubkey!("4wc5NC3ejYC5y4vQD4Eg3PsYW9ArZQV18fY1eHCJfxJQ");
const MINT_92C: Pubkey = pubkey!("92k8rwgvovnWXAC5eE7ASMekSuphLiA9iS16UsAHCdWo");
const MINT_SLUSD: Pubkey = pubkey!("92B5Ubkm5JBVGQeEeMnrkmmL6NpAGAirUua62BWAhZyJ");
const TOKEN_PROGRAM: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

// ABI `swap_sovereign_cpmm` (lib.rs L5252) — data (26 octets) :
//   disc(8) || amount_in u64 LE || min_amount_out u64 LE || seal_92 u16 LE
const SWAP_DISCRIMINATOR: [u8; 8] = [0x16, 0xa5, 0xc2, 0xdb, 0x47, 0x31, 0x8c, 0x34];
const SEAL_92: u16 = 0x3239; // b"92" LE

// Discriminateur Anchor du compte `SovereignPoolExtension`
// (= sha256("account:SovereignPoolExtension")[..8]).
const POOL_DISCRIMINATOR: [u8; 8] = [0xb9, 0x09, 0x11, 0xfc, 0xd4, 0x1b, 0x9f, 0x91];

// ── Test de parité : un `#[test]` par pool / état snapshotté ──────────────
#[test]
fn test_sterling_cpmm_92c_slusd() {
    let test = PoolTest::new(STERLING_POOL)
        .add_swap(MINT_92C, MINT_SLUSD, 100_000_000) // 1M 92C  -> SLUSD
        .add_swap(MINT_SLUSD, MINT_92C, 50_000_000); // 0.5M SLUSD -> 92C

    assert_pool_parity::<SterlingCpmmAmm>(&test, encode_swap);
}

// ── Encodeur natif (seul hook intégrateur : data d'instruction) ───────────
fn encode_swap(swap: &Swap, in_amount: u64) -> Vec<u8> {
    match swap {
        Swap::Placeholder { .. } => {
            let mut data = Vec::with_capacity(8 + 8 + 8 + 2);
            data.extend_from_slice(&SWAP_DISCRIMINATOR);
            data.extend_from_slice(&in_amount.to_le_bytes());
            data.extend_from_slice(&0u64.to_le_bytes()); // min_amount_out non contraint
            data.extend_from_slice(&SEAL_92.to_le_bytes());
            data
        }
        other => panic!("unexpected swap variant: {other:?}"),
    }
}

// ── Implémentation Amm (trait jupiter-amm-interface) ──────────────────────
#[derive(Clone)]
struct SterlingCpmmAmm {
    key: Pubkey,
    token_program: Pubkey,
    mint_souverain: Pubkey,
    mint_collateral: Pubkey,
    vault_a: Pubkey,
    vault_b: Pubkey,
    reserve_souverain: u64,
    reserve_collateral: u64,
}

single_program_amm!(SterlingCpmmAmm, STERLING_CPMM_PROGRAM, "Sterling Sovereign CPMM");

fn read_pubkey(d: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(d[offset..offset + 32].try_into().unwrap())
}

fn read_u64(d: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(d[offset..offset + 8].try_into().unwrap())
}

/// Courbe on-chain `cpmm_amount_out` (lib.rs L10863), bit-à-bit :
/// `out = (in * reserve_out) / (reserve_in + in)` — sans frais, arrondi bas.
fn cpmm_amount_out(amount_in: u64, reserve_in: u64, reserve_out: u64) -> Option<u64> {
    let numerator = (amount_in as u128).checked_mul(reserve_out as u128)?;
    let denominator = (reserve_in as u128).checked_add(amount_in as u128)?;
    if denominator == 0 {
        return None;
    }
    let out = numerator / denominator;
    if out > reserve_out as u128 {
        return None;
    }
    Some(out as u64)
}

impl Amm for SterlingCpmmAmm {
    fn from_keyed_account(
        keyed_account: &KeyedAccount,
        _amm_context: &AmmContext,
    ) -> Result<Self, AmmError> {
        if keyed_account.account.owner() != &STERLING_CPMM_PROGRAM {
            return Err(AmmError::from("pool is not owned by Sterling Sovereign CPMM"));
        }
        let d = keyed_account.account.data();
        // Layout `SovereignPoolExtension` (322 octets) — offsets symétriques de
        // `decode_sovereign_pool` (lib.rs L10786).
        if d.len() < 322 || d[..8] != POOL_DISCRIMINATOR {
            return Err(AmmError::from("unexpected sovereign pool account layout"));
        }
        Ok(Self {
            key: keyed_account.key,
            token_program: TOKEN_PROGRAM,
            mint_souverain: read_pubkey(d, 8),
            mint_collateral: read_pubkey(d, 40),
            vault_a: read_pubkey(d, 72),
            vault_b: read_pubkey(d, 104),
            reserve_souverain: read_u64(d, 176),
            reserve_collateral: read_u64(d, 184),
        })
    }

    fn label(&self) -> AmmLabel {
        Self::LABEL
    }

    fn program_id(&self) -> Pubkey {
        Self::PROGRAM_ID
    }

    fn key(&self) -> Pubkey {
        self.key
    }

    fn get_reserve_mints(&self) -> Vec<Pubkey> {
        vec![self.mint_souverain, self.mint_collateral]
    }

    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        // Le compte pool porte les réserves stockées ; les vaults servent au swap.
        vec![self.key, self.vault_a, self.vault_b]
    }

    fn update(&mut self, account_provider: impl AccountProvider) -> Result<(), AmmError> {
        let account = account_provider
            .get(&self.key)
            .ok_or_else(|| AmmError::from("missing pool account"))?;
        let d = account.data();
        if d.len() < 322 || d[..8] != POOL_DISCRIMINATOR {
            return Err(AmmError::from("unexpected sovereign pool account layout"));
        }
        // Le handler utilise les réserves STOCKÉES dans le compte pool (offsets
        // 176/184), PAS les balances des vaults — on lit donc les mêmes champs.
        self.reserve_souverain = read_u64(d, 176);
        self.reserve_collateral = read_u64(d, 184);
        Ok(())
    }

    fn quote(&self, quote_params: &QuoteParams) -> Result<Quote, AmmError> {
        let (reserve_in, reserve_out) = if quote_params.input_mint == self.mint_souverain {
            (self.reserve_souverain, self.reserve_collateral)
        } else {
            (self.reserve_collateral, self.reserve_souverain)
        };
        let out = cpmm_amount_out(quote_params.amount, reserve_in, reserve_out)
            .ok_or_else(|| AmmError::from("cpmm amount out overflow"))?;
        Ok(Quote {
            in_amount: quote_params.amount,
            out_amount: out,
            ..Default::default()
        })
    }

    fn get_swap_and_account_metas(
        &self,
        swap_params: &SwapParams,
    ) -> Result<SwapAndAccountMetas, AmmError> {
        // Ordre on-chain `SovereignSwapCpmm` (8 comptes, lib.rs L10653) :
        // [0] authority (signer) · [1] sovereign_pool · [2] pool_authority (= pool PDA)
        // [3] user_source · [4] user_destination · [5] vault_a · [6] vault_b · [7] token_program
        let account_metas = vec![
            AccountMeta::new_readonly(swap_params.token_transfer_authority, true),
            AccountMeta::new(self.key, false),
            AccountMeta::new_readonly(self.key, false), // pool_authority == sovereign_pool
            AccountMeta::new(swap_params.source_token_account, false),
            AccountMeta::new(swap_params.destination_token_account, false),
            AccountMeta::new(self.vault_a, false),
            AccountMeta::new(self.vault_b, false),
            AccountMeta::new_readonly(self.token_program, false),
        ];
        Ok(SwapAndAccountMetas {
            swap: Swap::Placeholder { data: None },
            account_metas,
        })
    }
}
