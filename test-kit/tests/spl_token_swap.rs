//! Reference parity test suite: an SPL Token Swap `Amm` verified against the real
//! on-chain program with the test-kit. This is the shape an integrator's `tests/` file
//! takes — register a `PoolTest` per pool/state and assert with `assert_pool_parity`.
//!
//!   cargo test -p jupiter-amm-test-kit --test spl_token_swap
//!
//! Fixture: a SOL/USDC constant-product pool (`AmHU…`) on the canonical SPL Token Swap
//! deployment (`SwaPpA9…`, built from `solana-labs/solana-program-library`), so both the
//! program source and the quote math are fully open. To refresh the fixtures, rerun with
//! `REFRESH=1 RPC=<mainnet-url>` set.
//!
//! The `Amm` impl below implements the public `jupiter-amm-interface` trait for SPL Token
//! Swap, with the constant-product + fee math vendored inline (the `spl-token-swap`
//! crate is stuck on an old Solana generation and won't compile here).

// The Solana types come from `jupiter_amm_interface::solana`, not from `solana-*`
// dependencies of this crate — that is all an `Amm` impl needs, and it is what keeps an
// integrator's tree on one Solana generation.
use {
    jupiter_amm_interface::{
        AccountProvider, Amm, AmmContext, AmmError, AmmLabel, KeyedAccount, Quote, QuoteParams,
        SingleProgramAmm, Swap, SwapAndAccountMetas, SwapParams, single_program_amm,
        solana::{
            account::ReadableAccount,
            instruction::AccountMeta,
            pubkey::{Pubkey, pubkey},
        },
    },
    jupiter_amm_test_kit::{PoolTest, assert_pool_parity},
};

const SPL_TOKEN_SWAP_PROGRAM: Pubkey = pubkey!("SwaPpA9LAaLfeLi3a68M4DjnLqgtticKg6CnyNwgAC8");
const SOL_USDC_POOL: Pubkey = pubkey!("AmHUjHKfSFP34D4VgPsviFNjWrvTN761Yazvv2eKAsSz");
const WSOL: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
const USDC: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

// =======================================================================================
// Registered pools — a test per pool (or per snapshotted state), each with a `#[test]`.
// =======================================================================================

#[test]
fn test_spl_token_swap_sol_usdc() {
    let test = PoolTest::new(SOL_USDC_POOL)
        .add_swap(WSOL, USDC, 100_000_000) // 0.1 SOL -> USDC
        .add_swap(USDC, WSOL, 50_000_000); // 50 USDC -> SOL

    assert_pool_parity::<SplTokenSwapAmm>(&test, encode_swap);
}

// =======================================================================================
// The AMM implementation and its native-swap encoder.
// =======================================================================================

/// The single integrator-supplied hook: native SPL Token Swap `Swap` instruction data —
/// discriminator `1`, `amount_in` (LE u64), `minimum_amount_out` (LE u64, left
/// unconstrained at 0 — asserting the exact out amount is the kit's job).
///
/// This program's instruction needs nothing beyond the amount, so its `data` is `None`
/// — a direction flag or an operation tag would go in there if the program needs one.
fn encode_swap(swap: &Swap, in_amount: u64) -> Vec<u8> {
    match swap {
        Swap::Placeholder { .. } => {
            let mut data = Vec::with_capacity(1 + 8 + 8);
            data.push(1);
            data.extend_from_slice(&in_amount.to_le_bytes());
            data.extend_from_slice(&0u64.to_le_bytes());
            data
        }
        other => panic!("unexpected swap variant: {other:?}"),
    }
}

/// Vendored SPL Token Swap constant-product + fee math (Apache-2.0), reproducing
/// `SwapCurve::swap` for the ConstantProduct curve bit-for-bit.
mod spl_curve {
    #[derive(Clone, Copy)]
    pub struct Fees {
        pub trade_fee_numerator: u64,
        pub trade_fee_denominator: u64,
        pub owner_trade_fee_numerator: u64,
        pub owner_trade_fee_denominator: u64,
    }

    fn calculate_fee(amount: u128, numerator: u128, denominator: u128) -> Option<u128> {
        if numerator == 0 || amount == 0 {
            Some(0)
        } else {
            let fee = amount.checked_mul(numerator)?.checked_div(denominator)?;
            Some(if fee == 0 { 1 } else { fee })
        }
    }

    fn checked_ceil_div(dividend: u128, mut rhs: u128) -> Option<(u128, u128)> {
        let mut quotient = dividend.checked_div(rhs)?;
        if quotient == 0 {
            return None;
        }
        if dividend.checked_rem(rhs)? > 0 {
            quotient = quotient.checked_add(1)?;
            rhs = dividend.checked_div(quotient)?;
            if dividend.checked_rem(quotient)? > 0 {
                rhs = rhs.checked_add(1)?;
            }
        }
        Some((quotient, rhs))
    }

    fn swap_without_fees(
        source_amount: u128,
        src_reserve: u128,
        dst_reserve: u128,
    ) -> Option<u128> {
        let invariant = src_reserve.checked_mul(dst_reserve)?;
        let new_src = src_reserve.checked_add(source_amount)?;
        let (new_dst, _adjusted_new_src) = checked_ceil_div(invariant, new_src)?;
        let dst_swapped = dst_reserve.checked_sub(new_dst)?;
        if dst_swapped == 0 {
            None
        } else {
            Some(dst_swapped)
        }
    }

    impl Fees {
        pub fn out_amount(
            &self,
            source_amount: u128,
            src_reserve: u128,
            dst_reserve: u128,
        ) -> Option<u128> {
            let trade_fee = calculate_fee(
                source_amount,
                self.trade_fee_numerator as u128,
                self.trade_fee_denominator as u128,
            )?;
            let owner_fee = calculate_fee(
                source_amount,
                self.owner_trade_fee_numerator as u128,
                self.owner_trade_fee_denominator as u128,
            )?;
            let source_less_fees = source_amount.checked_sub(trade_fee.checked_add(owner_fee)?)?;
            swap_without_fees(source_less_fees, src_reserve, dst_reserve)
        }
    }
}

fn read_pubkey(d: &[u8], off: usize) -> Pubkey {
    Pubkey::try_from(&d[off..off + 32]).unwrap()
}
fn read_u64(d: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(d[off..off + 8].try_into().unwrap())
}

#[derive(Clone)]
struct SplTokenSwapAmm {
    key: Pubkey,
    authority: Pubkey,
    token_a: Pubkey,
    token_b: Pubkey,
    token_a_mint: Pubkey,
    token_b_mint: Pubkey,
    pool_mint: Pubkey,
    pool_fee_account: Pubkey,
    token_program_id: Pubkey,
    fees: spl_curve::Fees,
    reserves: [u128; 2],
}

single_program_amm!(SplTokenSwapAmm, SPL_TOKEN_SWAP_PROGRAM, "SPL Token Swap");

impl Amm for SplTokenSwapAmm {
    fn from_keyed_account(
        keyed_account: &KeyedAccount,
        _amm_context: &AmmContext,
    ) -> Result<Self, AmmError> {
        if keyed_account.account.owner() != &Self::PROGRAM_ID {
            return Err(AmmError::from("pool is not owned by SPL Token Swap"));
        }
        let d = &keyed_account.account.data()[1..]; // skip version byte
        if d[290] != 0 {
            return Err(AmmError::from(
                "example only supports the constant-product curve",
            ));
        }
        Ok(Self {
            key: keyed_account.key,
            authority: Pubkey::find_program_address(
                &[&keyed_account.key.to_bytes()],
                &Self::PROGRAM_ID,
            )
            .0,
            token_program_id: read_pubkey(d, 2),
            token_a: read_pubkey(d, 34),
            token_b: read_pubkey(d, 66),
            pool_mint: read_pubkey(d, 98),
            token_a_mint: read_pubkey(d, 130),
            token_b_mint: read_pubkey(d, 162),
            pool_fee_account: read_pubkey(d, 194),
            fees: spl_curve::Fees {
                trade_fee_numerator: read_u64(d, 226),
                trade_fee_denominator: read_u64(d, 234),
                owner_trade_fee_numerator: read_u64(d, 242),
                owner_trade_fee_denominator: read_u64(d, 250),
            },
            reserves: [0, 0],
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
        vec![self.token_a_mint, self.token_b_mint]
    }
    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        vec![self.token_a, self.token_b]
    }

    fn update(&mut self, account_provider: impl AccountProvider) -> Result<(), AmmError> {
        let amount = |vault: &Pubkey| -> Result<u128, AmmError> {
            let account = account_provider
                .get(vault)
                .ok_or_else(|| AmmError::from("missing vault account"))?;
            Ok(u64::from_le_bytes(account.data()[64..72].try_into().unwrap()) as u128)
        };
        self.reserves = [amount(&self.token_a)?, amount(&self.token_b)?];
        Ok(())
    }

    fn quote(&self, quote_params: &QuoteParams) -> Result<Quote, AmmError> {
        let (src_reserve, dst_reserve) = if quote_params.input_mint == self.token_a_mint {
            (self.reserves[0], self.reserves[1])
        } else {
            (self.reserves[1], self.reserves[0])
        };
        let out = self
            .fees
            .out_amount(quote_params.amount as u128, src_reserve, dst_reserve)
            .ok_or_else(|| AmmError::from("swap math failed"))?;
        Ok(Quote {
            in_amount: quote_params.amount,
            out_amount: out as u64,
            ..Default::default()
        })
    }

    fn get_swap_and_account_metas(
        &self,
        swap_params: &SwapParams,
    ) -> Result<SwapAndAccountMetas, AmmError> {
        let (swap_source, swap_destination) = if swap_params.source_mint == self.token_a_mint {
            (self.token_a, self.token_b)
        } else {
            (self.token_b, self.token_a)
        };
        // Canonical SPL Token Swap `Swap` account order; `user_transfer_authority` — the
        // swapping user — signs the top-level instruction.
        let account_metas = vec![
            AccountMeta::new_readonly(self.key, false),
            AccountMeta::new_readonly(self.authority, false),
            AccountMeta::new_readonly(swap_params.token_transfer_authority, true),
            AccountMeta::new(swap_params.source_token_account, false),
            AccountMeta::new(swap_source, false),
            AccountMeta::new(swap_destination, false),
            AccountMeta::new(swap_params.destination_token_account, false),
            AccountMeta::new(self.pool_mint, false),
            AccountMeta::new(self.pool_fee_account, false),
            AccountMeta::new_readonly(self.token_program_id, false),
        ];
        Ok(SwapAndAccountMetas {
            swap: Swap::Placeholder { data: None },
            account_metas,
        })
    }
}
