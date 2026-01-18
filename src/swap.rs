use anyhow::anyhow;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Swap {
    Saber,
    SaberAddDecimalsDeposit,
    SaberAddDecimalsWithdraw,
    TokenSwap,
    Raydium,
    Crema {
        a_to_b: bool,
    },
    Mercurial,
    Aldrin {
        side: Side,
    },
    AldrinV2 {
        side: Side,
    },
    Whirlpool {
        a_to_b: bool,
    },
    Invariant {
        x_to_y: bool,
    },
    Meteora,
    MarcoPolo {
        x_to_y: bool,
    },
    LifinityV2,
    RaydiumClmm,
    Phoenix {
        side: Side,
    },
    TokenSwapV2,
    HeliumTreasuryManagementRedeemV0,
    StakeDexStakeWrappedSol,
    MeteoraDlmm,
    OpenBookV2 {
        side: Side,
    },
    RaydiumClmmV2,
    StakeDexPrefundWithdrawStakeAndDepositStake {
        bridge_stake_seed: u32,
    },
    SanctumS {
        src_lst_value_calc_accs: u8,
        dst_lst_value_calc_accs: u8,
        src_lst_index: u32,
        dst_lst_index: u32,
    },
    SanctumSAddLiquidity {
        lst_value_calc_accs: u8,
        lst_index: u32,
    },
    SanctumSRemoveLiquidity {
        lst_value_calc_accs: u8,
        lst_index: u32,
    },
    RaydiumCP,
    WhirlpoolSwapV2 {
        a_to_b: bool,
        remaining_accounts_info: Option<RemainingAccountsInfo>,
    },
    OneIntro,
    PerpsV2,
    PerpsV2AddLiquidity,
    PerpsV2RemoveLiquidity,
    MoonshotWrappedBuy,
    MoonshotWrappedSell,
    StabbleStableSwap,
    StabbleWeightedSwap,
    Obric {
        x_to_y: bool,
    },
    SolFi {
        is_quote_to_base: bool,
    },
    SolayerDelegateNoInit,
    SolayerUndelegateNoInit,
    ZeroFi,
    StakeDexWithdrawWrappedSol,
    VirtualsBuy,
    VirtualsSell,
    Perena {
        in_index: u8,
        out_index: u8,
    },
    Gamma,
    MeteoraDlmmSwapV2 {
        remaining_accounts_info: RemainingAccountsInfo,
    },
    Woofi,
    MeteoraDammV2,
    StabbleStableSwapV2,
    StabbleWeightedSwapV2,
    RaydiumLaunchlabBuy {
        share_fee_rate: u64,
    },
    RaydiumLaunchlabSell {
        share_fee_rate: u64,
    },
    BoopdotfunWrappedBuy,
    BoopdotfunWrappedSell,
    Plasma {
        side: Side,
    },
    GoonFi {
        is_bid: bool,
        blacklist_bump: u8,
    },
    HumidiFi {
        swap_id: u64,
        is_base_to_quote: bool,
    },
    MeteoraDynamicBondingCurveSwapWithRemainingAccounts,
    TesseraV {
        side: Side,
    },
    Heaven {
        a_to_b: bool,
    },
    SolFiV2 {
        is_quote_to_base: bool,
    },
    Aquifer,
    PumpWrappedBuyV3,
    PumpWrappedSellV3,
    PumpSwapBuyV3,
    PumpSwapSellV3,
    JupiterLendDeposit,
    JupiterLendRedeem,
    DefiTuna {
        a_to_b: bool,
        remaining_accounts_info: Option<RemainingAccountsInfo>,
    },
    AlphaQ {
        a_to_b: bool,
    },
    RaydiumV2,
    SarosDlmm {
        swap_for_y: bool,
    },
    Futarchy {
        side: Side,
    },
    MeteoraDammV2WithRemainingAccounts,
    Obsidian,
    WhaleStreet {
        side: Side,
    },
    DynamicV1 {
        candidate_swaps: Vec<CandidateSwap>,
    },
    PumpWrappedBuyV4,
    PumpWrappedSellV4,
    CarrotIssue,
    CarrotRedeem,
    Manifest {
        side: Side,
    },
    BisonFi {
        a_to_b: bool,
    },
    HumidiFiV2 {
        swap_id: u64,
        is_base_to_quote: bool,
    },
    PerenaStar {
        is_mint: bool,
    },
    GoonFiV2 {
        is_bid: bool,
    },
}

impl Swap {
    /// SOVEREIGN OPTIMIZATION: Check if the AMM physically supports the requested SwapMode.
    pub fn is_mode_supported(&self, mode: crate::SwapMode) -> bool {
        match (self, mode) {
            (Swap::Saber, _) => true,
            (Swap::Raydium, _) => true,
            _ => true, // Default to true for most, but allows for specific filtering
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AccountsType {
    TransferHookA,
    TransferHookB,
    // TransferHookReward,
    // TransferHookInput,
    // TransferHookIntermediate,
    // TransferHookOutput,
    TickArray,
    // TickArrayOne,
    // TickArrayTwo,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemainingAccountsSlice {
    pub accounts_type: AccountsType,
    pub length: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemainingAccountsInfo {
    pub slices: Vec<RemainingAccountsSlice>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum CandidateSwap {
    HumidiFi {
        swap_id: u64,
        is_base_to_quote: bool,
    },
    TesseraV {
        side: Side,
    },
    HumidiFiV2 {
        swap_id: u64,
        is_base_to_quote: bool,
    },
}

impl TryInto<CandidateSwap> for Swap {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<CandidateSwap, Self::Error> {
        let candidate_swap = match self {
            Swap::HumidiFi {
                swap_id,
                is_base_to_quote,
            } => CandidateSwap::HumidiFi {
                swap_id,
                is_base_to_quote,
            },
            Swap::TesseraV { side } => CandidateSwap::TesseraV { side },
            Swap::HumidiFiV2 {
                swap_id,
                is_base_to_quote,
            } => CandidateSwap::HumidiFiV2 {
                swap_id,
                is_base_to_quote,
            },
            _ => return Err(anyhow!("Swap {self:?} is not a valid candidate swap")),
        };
        Ok(candidate_swap)
    }
}
