use {
    rust_decimal::Decimal,
    serde::{Deserialize, Serialize},
    solana_account::{Account, ReadableAccount},
    solana_clock::Clock,
    solana_instruction::AccountMeta,
    solana_pubkey::Pubkey,
    std::{
        collections::{HashMap, HashSet},
        hash::BuildHasher,
        ops::Deref,
        str::FromStr,
        sync::{
            Arc,
            atomic::{AtomicI64, AtomicU64, Ordering},
        },
    },
    thiserror::Error,
};

pub mod serde_utils;
mod swap;
pub use swap::{
    AccountsType, CandidateSwap, RemainingAccountsInfo, RemainingAccountsSlice, Side, Swap,
};

#[derive(Serialize, Deserialize, PartialEq, Clone, Copy, Default, Debug)]
pub enum SwapMode {
    #[default]
    ExactIn,
    ExactOut,
}

impl FromStr for SwapMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "ExactIn" => Ok(SwapMode::ExactIn),
            "ExactOut" => Ok(SwapMode::ExactOut),
            _ => Err(anyhow::anyhow!("{} is not a valid SwapMode", s)),
        }
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub enum FeeMode {
    #[default]
    Normal,
    Ultra,
}

#[derive(Debug)]
pub struct QuoteParams {
    pub amount: u64,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub swap_mode: SwapMode,
    pub fee_mode: FeeMode,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Quote {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_mint: Pubkey,
    pub fee_pct: Decimal,
}

pub type QuoteMintToReferrer = HashMap<Pubkey, Pubkey, ahash::RandomState>;

pub struct SwapParams<'a, 'b> {
    pub swap_mode: SwapMode,
    pub in_amount: u64,
    pub out_amount: u64,
    pub source_mint: Pubkey,
    pub destination_mint: Pubkey,
    pub source_token_account: Pubkey,
    pub destination_token_account: Pubkey,
    /// This can be the user or the program authority over the source_token_account.
    pub token_transfer_authority: Pubkey,
    /// The actual user doing the swap.
    pub user: Pubkey,
    /// The payer for extra SOL that is required for needed accounts in the swap.
    pub payer: Pubkey,
    pub quote_mint_to_referrer: Option<&'a QuoteMintToReferrer>,
    pub jupiter_program_id: &'b Pubkey,
    /// Instead of returning the relevant Err, replace dynamic accounts with the default Pubkey
    /// This is useful for crawling market with no tick array
    pub missing_dynamic_accounts_as_default: bool,
}

impl SwapParams<'_, '_> {
    /// A placeholder to indicate an optional account or used as a terminator when consuming remaining accounts
    /// Using the jupiter program id
    pub fn placeholder_account_meta(&self) -> AccountMeta {
        AccountMeta::new_readonly(*self.jupiter_program_id, false)
    }
}

pub struct SwapAndAccountMetas {
    pub swap: Swap,
    pub account_metas: Vec<AccountMeta>,
}

#[derive(Debug, Error)]
#[error("Could not find address: {0}")]
pub struct AccountNotFoundError(Pubkey);

pub trait AccountProvider {
    fn get(&self, pubkey: &Pubkey) -> Option<impl ReadableAccount + use<'_, Self>>;

    fn try_get(&self, pubkey: &Pubkey) -> Result<impl ReadableAccount, AccountNotFoundError> {
        self.get(pubkey).ok_or(AccountNotFoundError(*pubkey))
    }
}

impl<V, S: BuildHasher> AccountProvider for HashMap<Pubkey, V, S>
where
    V: Deref,
    V::Target: ReadableAccount,
{
    fn get(&self, pubkey: &Pubkey) -> Option<impl ReadableAccount + use<'_, V, S>> {
        HashMap::get(self, pubkey).map(Deref::deref)
    }
}

pub trait Amm: Clone {
    fn from_keyed_account(
        keyed_account: &KeyedAccount,
        amm_context: &AmmContext,
    ) -> anyhow::Result<Self>
    where
        Self: Sized;

    /// A human readable label of the underlying DEX
    fn label(&self) -> String;

    fn program_id(&self) -> Pubkey;

    /// The pool state or market state address
    fn key(&self) -> Pubkey;

    /// The mints that can be traded
    fn get_reserve_mints(&self) -> Vec<Pubkey>;

    /// The accounts necessary to produce a quote
    fn get_accounts_to_update(&self) -> Vec<Pubkey>;

    /// Picks necessary accounts to update it's internal state
    /// Heavy deserialization and precomputation caching should be done in this function
    fn update(&mut self, account_provider: impl AccountProvider) -> anyhow::Result<()>;

    fn quote(&self, quote_params: &QuoteParams) -> anyhow::Result<Quote>;

    /// Indicates which Swap has to be performed along with all the necessary account metas
    fn get_swap_and_account_metas(
        &self,
        swap_params: &SwapParams,
    ) -> anyhow::Result<SwapAndAccountMetas>;

    /// Indicates if get_accounts_to_update might return a non constant vec
    fn has_dynamic_accounts(&self) -> bool {
        false
    }

    /// Indicates whether `update` needs to be called before `get_reserve_mints`
    fn requires_update_for_reserve_mints(&self) -> bool {
        false
    }

    // Indicates that whether ExactOut mode is supported
    fn supports_exact_out(&self) -> bool {
        false
    }

    /// It can only trade in one direction from its first mint to second mint, assuming it is a two mint AMM
    fn unidirectional(&self) -> bool {
        false
    }

    /// For testing purposes, provide a mapping of dependency programs to function
    fn program_dependencies(&self) -> Vec<(Pubkey, String)> {
        vec![]
    }

    fn get_accounts_len(&self) -> usize {
        32 // Default to a near whole legacy transaction to penalize no implementation
    }

    /// The identifier of the underlying liquidity
    ///
    /// Example:
    /// For RaydiumAmm uses Openbook market A this will return Some(A)
    /// For Openbook market A, it will also return Some(A)
    fn underlying_liquidities(&self) -> Option<HashSet<Pubkey>> {
        None
    }

    /// Provides a shortcut to establish if the AMM can be used for trading
    /// If the market is active at all
    fn is_active(&self) -> bool {
        true
    }
}

pub type AmmLabel = &'static str;

pub trait AmmProgramIdToLabel {
    const PROGRAM_ID_TO_LABELS: &[(Pubkey, AmmLabel)];
}

pub trait SingleProgramAmm {
    const PROGRAM_ID: Pubkey;
    const LABEL: AmmLabel;
}

impl<T: SingleProgramAmm> AmmProgramIdToLabel for T {
    const PROGRAM_ID_TO_LABELS: &[(Pubkey, AmmLabel)] = &[(Self::PROGRAM_ID, Self::LABEL)];
}

#[macro_export]
macro_rules! single_program_amm {
    ($amm_struct:ty, $program_id:expr, $label:expr) => {
        impl SingleProgramAmm for $amm_struct {
            const PROGRAM_ID: Pubkey = $program_id;
            const LABEL: &'static str = $label;
        }
    };
}

#[derive(Clone, Deserialize, Serialize)]
pub struct KeyedAccount {
    pub key: Pubkey,
    pub account: Account,
    pub params: Option<serde_json::Value>,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    #[serde(with = "serde_utils::field_as_string")]
    pub pubkey: Pubkey,
    #[serde(with = "serde_utils::field_as_string")]
    pub owner: Pubkey,
    /// Additional data an Amm requires, Amm dependent and decoded in the Amm implementation
    pub params: Option<serde_json::Value>,
}

impl From<KeyedAccount> for Market {
    fn from(
        KeyedAccount {
            key,
            account,
            params,
        }: KeyedAccount,
    ) -> Self {
        Market {
            pubkey: key,
            owner: account.owner,
            params,
        }
    }
}

#[derive(Default)]
pub struct AmmContext {
    pub clock_ref: ClockRef,
}

#[derive(Default)]
pub struct ClockData {
    pub slot: AtomicU64,
    /// The timestamp of the first `Slot` in this `Epoch`.
    pub epoch_start_timestamp: AtomicI64,
    /// The current `Epoch`.
    pub epoch: AtomicU64,
    pub leader_schedule_epoch: AtomicU64,
    pub unix_timestamp: AtomicI64,
}

#[derive(Default, Clone)]
pub struct ClockRef(Arc<ClockData>);

impl Deref for ClockRef {
    type Target = ClockData;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ClockRef {
    pub fn update(&self, clock: Clock) {
        self.epoch.store(clock.epoch, Ordering::Relaxed);
        self.slot.store(clock.slot, Ordering::Relaxed);
        self.unix_timestamp
            .store(clock.unix_timestamp, Ordering::Relaxed);
        self.epoch_start_timestamp
            .store(clock.epoch_start_timestamp, Ordering::Relaxed);
        self.leader_schedule_epoch
            .store(clock.leader_schedule_epoch, Ordering::Relaxed);
    }
}

impl From<Clock> for ClockRef {
    fn from(clock: Clock) -> Self {
        ClockRef(Arc::new(ClockData {
            epoch: AtomicU64::new(clock.epoch),
            epoch_start_timestamp: AtomicI64::new(clock.epoch_start_timestamp),
            leader_schedule_epoch: AtomicU64::new(clock.leader_schedule_epoch),
            slot: AtomicU64::new(clock.slot),
            unix_timestamp: AtomicI64::new(clock.unix_timestamp),
        }))
    }
}

#[cfg(test)]
mod tests {
    use {super::*, solana_pubkey::pubkey};

    #[test]
    fn test_market_deserialization() {
        let json = r#"
        {
            "lamports": 1000,
            "owner": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "pubkey": "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263",
            "executable": false,
            "rentEpoch": 0
        }
        "#;
        let market: Market = serde_json::from_str(json).unwrap();
        assert_eq!(
            market.owner,
            pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
        );
        assert_eq!(
            market.pubkey,
            pubkey!("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263")
        );
    }
}
