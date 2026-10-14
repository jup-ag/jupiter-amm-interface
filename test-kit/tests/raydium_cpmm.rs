//! Exercise native ExactIn and ExactOut instructions against a snapshotted Raydium
//! CPMM SOL/USDC pool. See fixtures/raydium_cpmm.json for fixture provenance.
//! The small adapter covers SPL Token and Token-2022 transfer fees, constant trade fees,
//! and pools without creator fees. It is a test fixture, not a production Raydium SDK.

use std::sync::atomic::Ordering;

use jupiter_amm_interface::{
    AccountProvider, Amm, AmmContext, AmmError, AmmLabel, KeyedAccount, Quote, QuoteParams, Swap,
    SwapAndAccountMetas, SwapMode, SwapParams,
};
use jupiter_amm_test_kit::{
    PoolSnapshot, PoolTest, assert_pool_parity, assert_pool_parity_with_swap_params,
};
use solana_account::ReadableAccount;
use solana_instruction::AccountMeta;
use solana_pubkey::{Pubkey, pubkey};

const PROGRAM: Pubkey = pubkey!("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");
const POOL: Pubkey = pubkey!("7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny");
const WSOL: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
const USDC: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const TOKEN_PROGRAM: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const TOKEN_2022: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
const FEE_POOL: Pubkey = pubkey!("5odbv2a15FG6vtfnJRwQM2pYC8D6Q3jHqE6aj7NgQJ4N");
const FEE_MINT: Pubkey = pubkey!("H78X8fzErUhmr9XoBymqGnWU67rkReP1mvxHrA1vLxfN");
const FEE_DENOMINATOR: u128 = 1_000_000;

fn exact_out_test() -> PoolTest {
    PoolTest::new(POOL).add_exact_out_swap(WSOL, USDC, 1_000_000)
}

#[test]
fn native_exact_in_and_exact_out_both_directions() {
    let test = PoolTest::new(POOL)
        .add_swap(WSOL, USDC, 100_000_000)
        .add_swap(USDC, WSOL, 50_000_000)
        .add_exact_out_swap(WSOL, USDC, 1_000_000)
        .add_exact_out_swap(USDC, WSOL, 100_000_000)
        .add_exact_out_swap(WSOL, USDC, 1)
        .add_exact_out_swap(USDC, WSOL, 1);
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
fn native_token_2022_transfer_fees_both_directions_and_caps() {
    // The live mint charges 7%, capped at 1,000,000,000 atoms. These amounts
    // exercise fees below and at the cap, on both the input and output sides.
    let snapshot = PoolSnapshot::load_dir(PoolTest::new(FEE_POOL).fixtures_dir).unwrap();
    let mint = snapshot.get(&FEE_MINT).unwrap();
    assert_eq!(*mint.owner(), TOKEN_2022);
    let fee = TransferFee::from_mint(mint.data(), snapshot.clock().unwrap().epoch).unwrap();
    assert_eq!((fee.basis_points, fee.maximum), (700, 1_000_000_000));
    assert_eq!(fee.fee(1_000_000_000), 70_000_000);
    assert_eq!(fee.fee(100_000_000_000), fee.maximum);
    assert_eq!(fee.gross(100_000_000_000) - 100_000_000_000, fee.maximum);
    let test = PoolTest::new(FEE_POOL)
        .add_swap(WSOL, FEE_MINT, 1_000_000)
        .add_swap(WSOL, FEE_MINT, 10_000_000)
        .add_swap(FEE_MINT, WSOL, 1_000_000_000)
        .add_swap(FEE_MINT, WSOL, 100_000_000_000)
        .add_exact_out_swap(WSOL, FEE_MINT, 1)
        .add_exact_out_swap(WSOL, FEE_MINT, 1_000_000_000)
        .add_exact_out_swap(WSOL, FEE_MINT, 100_000_000_000)
        .add_exact_out_swap(FEE_MINT, WSOL, 1)
        .add_exact_out_swap(FEE_MINT, WSOL, 1_000_000)
        .add_exact_out_swap(FEE_MINT, WSOL, 100_000_000);
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
#[should_panic(expected = "native swap transaction succeeds")]
fn detects_ignored_token_2022_input_fee() {
    let mut test = PoolTest::new(FEE_POOL).add_exact_out_swap(FEE_MINT, WSOL, 1_000_000);
    test.params = Some(serde_json::json!({"ignore_transfer_fees": true}));
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
#[should_panic(expected = "native swap transaction succeeds")]
fn detects_ignored_token_2022_output_fee() {
    let mut test = PoolTest::new(FEE_POOL).add_exact_out_swap(WSOL, FEE_MINT, 1_000_000_000);
    test.params = Some(serde_json::json!({"ignore_transfer_fees": true}));
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
#[should_panic(expected = "quote.in_amount does not match the on-chain input debit")]
fn detects_overquoted_token_2022_input() {
    let mut test = PoolTest::new(FEE_POOL).add_exact_out_swap(FEE_MINT, WSOL, 1_000_000);
    test.params = Some(serde_json::json!({"input_delta": 1}));
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
#[should_panic(expected = "quote.in_amount does not match the on-chain input debit")]
fn detects_overquoted_exact_out_input() {
    let mut test = exact_out_test();
    test.params = Some(serde_json::json!({"input_delta": 1}));
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
#[should_panic(expected = "native swap transaction succeeds")]
fn detects_underquoted_exact_out_input() {
    let mut test = exact_out_test();
    test.params = Some(serde_json::json!({"input_delta": -1}));
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
fn native_exact_out_with_permissive_input_limit() {
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(
        &exact_out_test(),
        encode_with_permissive_input_limit,
    );
}

#[test]
#[should_panic(expected = "native swap transaction succeeds")]
fn detects_underquoted_input_even_with_permissive_input_limit() {
    let mut test = exact_out_test();
    test.params = Some(serde_json::json!({"input_delta": -1}));
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_with_permissive_input_limit);
}

#[test]
#[should_panic(expected = "quote.out_amount does not match the on-chain output delta")]
fn detects_wrong_encoded_output() {
    // One and two wSOL atoms have the same rounded input cost in this snapshot, so
    // the input-debit assertion cannot mask a missing output-delta assertion.
    let test = PoolTest::new(POOL).add_exact_out_swap(USDC, WSOL, 2);
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, |swap, params| {
        let mut data = encode_swap(swap, params);
        data[16..24].copy_from_slice(&(params.out_amount - 1).to_le_bytes());
        data
    });
}

#[test]
#[should_panic(expected = "ExactOut quote.out_amount does not match the requested output amount")]
fn rejects_quote_for_a_different_output() {
    let mut test = exact_out_test();
    // The adapter quotes the correct input for the wrong target. Without the
    // requested-output guard, both quoted balance deltas would pass.
    test.params = Some(serde_json::json!({"output_delta": 1}));
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
#[should_panic(expected = "AMM does not support ExactOut")]
fn rejects_unsupported_exact_out() {
    let mut test = exact_out_test();
    test.params = Some(serde_json::json!({"supports_exact_out": false}));
    assert_pool_parity_with_swap_params::<RaydiumCpmm>(&test, encode_swap);
}

#[test]
#[should_panic(expected = "ExactOut swaps require assert_pool_parity_with_swap_params")]
fn legacy_encoder_rejects_exact_out() {
    assert_pool_parity::<RaydiumCpmm>(&exact_out_test(), |_, _| unreachable!());
}

// Anchor instruction discriminators for Raydium's native swap instructions. The second
// argument is a minimum output for ExactIn, and an exact output for ExactOut.
fn encode_swap(_swap: &Swap, params: &SwapParams<'_, '_>) -> Vec<u8> {
    let discriminator = match params.swap_mode {
        SwapMode::ExactIn => [143, 190, 90, 218, 196, 30, 51, 222],
        SwapMode::ExactOut => [55, 217, 98, 86, 163, 74, 180, 173],
    };
    let mut data = discriminator.to_vec();
    data.extend_from_slice(&params.in_amount.to_le_bytes());
    data.extend_from_slice(&params.out_amount.to_le_bytes());
    data
}

// Isolate the harness's source-account funding cap from the native instruction's
// slippage limit: the positive control proves this encoding is otherwise valid.
fn encode_with_permissive_input_limit(swap: &Swap, params: &SwapParams<'_, '_>) -> Vec<u8> {
    assert_eq!(params.swap_mode, SwapMode::ExactOut);
    let mut data = encode_swap(swap, params);
    data[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    data
}

fn read_pubkey(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::try_from(&data[offset..offset + 32]).unwrap()
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

#[derive(Clone, Copy, Default)]
struct TransferFee {
    basis_points: u128,
    maximum: u128,
}

impl TransferFee {
    fn from_mint(data: &[u8], epoch: u64) -> Result<Self, AmmError> {
        if data.len() == 82 {
            return Ok(Self::default());
        }
        // Token-2022 mint extensions follow the padded base state and account type.
        let mut offset = 166;
        let mut fee = Self::default();
        while offset + 4 <= data.len() {
            let kind = u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap());
            let len = usize::from(u16::from_le_bytes(
                data[offset + 2..offset + 4].try_into().unwrap(),
            ));
            offset += 4;
            let extension = data
                .get(offset..offset + len)
                .ok_or_else(|| AmmError::from("invalid mint extension"))?;
            match kind {
                0 => break,
                1 if len == 108 => {
                    // TransferFeeConfig: two authorities, withheld amount, then
                    // older/newer (epoch, maximum_fee, basis_points) records.
                    let record = if epoch >= read_u64(extension, 90) {
                        90
                    } else {
                        72
                    };
                    fee = Self {
                        maximum: u128::from(read_u64(extension, record + 8)),
                        basis_points: u128::from(u16::from_le_bytes(
                            extension[record + 16..record + 18].try_into().unwrap(),
                        )),
                    };
                }
                18 | 19 => {} // MetadataPointer / TokenMetadata do not change transfers.
                _ => return Err("fixture adapter does not support this mint extension".into()),
            }
            offset += len;
        }
        Ok(fee)
    }

    fn fee(self, gross: u128) -> u128 {
        (gross * self.basis_points)
            .div_ceil(10_000)
            .min(self.maximum)
    }

    fn gross(self, net: u128) -> u128 {
        let fee = if self.basis_points == 10_000 {
            self.maximum
        } else {
            (net * self.basis_points)
                .div_ceil(10_000 - self.basis_points)
                .min(self.maximum)
        };
        net + fee
    }
}

#[derive(Clone)]
struct RaydiumCpmm {
    key: Pubkey,
    config: Pubkey,
    vaults: [Pubkey; 2],
    mints: [Pubkey; 2],
    token_programs: [Pubkey; 2],
    transfer_fees: [TransferFee; 2],
    epoch: u64,
    ignore_transfer_fees: bool,
    observation: Pubkey,
    reserved_fees: [u64; 2],
    reserves: [u128; 2],
    trade_fee_rate: u128,
    input_delta: i64,
    output_delta: u64,
    supports_exact_out: bool,
    verify_exact_out_params: bool,
}

impl Amm for RaydiumCpmm {
    fn from_keyed_account(account: &KeyedAccount, context: &AmmContext) -> Result<Self, AmmError> {
        let data = account.account.data();
        if account.account.owner() != &PROGRAM || data.len() != 637 {
            return Err("expected a Raydium CPMM pool account".into());
        }
        if data[390] != 0 {
            return Err("fixture adapter does not support creator fees".into());
        }
        let token_programs = [read_pubkey(data, 232), read_pubkey(data, 264)];
        if token_programs
            .iter()
            .any(|p| ![TOKEN_PROGRAM, TOKEN_2022].contains(p))
        {
            return Err("unsupported token program".into());
        }
        let params = account.params.as_ref();
        Ok(Self {
            key: account.key,
            config: read_pubkey(data, 8),
            vaults: [read_pubkey(data, 72), read_pubkey(data, 104)],
            mints: [read_pubkey(data, 168), read_pubkey(data, 200)],
            token_programs,
            transfer_fees: [TransferFee::default(); 2],
            epoch: context.clock_ref.epoch.load(Ordering::Relaxed),
            ignore_transfer_fees: params
                .and_then(|p| p["ignore_transfer_fees"].as_bool())
                .unwrap_or(false),
            observation: read_pubkey(data, 296),
            reserved_fees: [
                read_u64(data, 341) + read_u64(data, 357) + read_u64(data, 397),
                read_u64(data, 349) + read_u64(data, 365) + read_u64(data, 405),
            ],
            reserves: [0, 0],
            trade_fee_rate: 0,
            input_delta: params.and_then(|p| p["input_delta"].as_i64()).unwrap_or(0),
            output_delta: params.and_then(|p| p["output_delta"].as_u64()).unwrap_or(0),
            supports_exact_out: params
                .and_then(|p| p["supports_exact_out"].as_bool())
                .unwrap_or(true),
            verify_exact_out_params: params
                .and_then(|p| p["verify_exact_out_params"].as_bool())
                .unwrap_or(false),
        })
    }

    fn label(&self) -> AmmLabel {
        "Raydium CPMM"
    }

    fn program_id(&self) -> Pubkey {
        PROGRAM
    }

    fn key(&self) -> Pubkey {
        self.key
    }

    fn get_reserve_mints(&self) -> Vec<Pubkey> {
        self.mints.to_vec()
    }

    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        vec![
            self.config,
            self.vaults[0],
            self.vaults[1],
            self.mints[0],
            self.mints[1],
        ]
    }

    fn update(&mut self, accounts: impl AccountProvider) -> Result<(), AmmError> {
        let config = accounts.try_get(&self.config)?;
        self.trade_fee_rate = u128::from(read_u64(config.data(), 12));
        for i in 0..2 {
            let vault = accounts.try_get(&self.vaults[i])?;
            self.reserves[i] = u128::from(read_u64(vault.data(), 64) - self.reserved_fees[i]);
            if self.token_programs[i] == TOKEN_2022 && !self.ignore_transfer_fees {
                let mint = accounts.try_get(&self.mints[i])?;
                self.transfer_fees[i] = TransferFee::from_mint(mint.data(), self.epoch)?;
            }
        }
        Ok(())
    }

    fn supports_exact_out(&self) -> bool {
        self.supports_exact_out
    }

    fn quote(&self, params: &QuoteParams) -> Result<Quote, AmmError> {
        let direction = self.direction(params.input_mint, params.output_mint)?;
        let (reserve_in, reserve_out) = (self.reserves[direction], self.reserves[1 - direction]);
        let amount = u128::from(params.amount)
            + if params.swap_mode == SwapMode::ExactOut {
                u128::from(self.output_delta)
            } else {
                0
            };
        let (input, output) = match params.swap_mode {
            SwapMode::ExactIn => {
                let received = amount - self.transfer_fees[direction].fee(amount);
                let fee = (received * self.trade_fee_rate).div_ceil(FEE_DENOMINATOR);
                let net = received - fee;
                let gross_output = net * reserve_out / (reserve_in + net);
                (
                    amount,
                    gross_output - self.transfer_fees[1 - direction].fee(gross_output),
                )
            }
            SwapMode::ExactOut => {
                let gross_output = self.transfer_fees[1 - direction].gross(amount);
                if gross_output >= reserve_out {
                    return Err("requested output exceeds the reserve".into());
                }
                let net = (reserve_in * gross_output).div_ceil(reserve_out - gross_output);
                (
                    self.transfer_fees[direction].gross(
                        (net * FEE_DENOMINATOR).div_ceil(FEE_DENOMINATOR - self.trade_fee_rate),
                    ),
                    amount,
                )
            }
        };
        let in_amount = u64::try_from(input)
            .map_err(|_| AmmError::from("input overflow"))?
            .checked_add_signed(self.input_delta)
            .ok_or_else(|| AmmError::from("mutated input overflow"))?;
        Ok(Quote {
            in_amount,
            out_amount: u64::try_from(output).unwrap()
                + if params.swap_mode == SwapMode::ExactIn {
                    self.output_delta
                } else {
                    0
                },
            ..Default::default()
        })
    }

    fn get_swap_and_account_metas(
        &self,
        params: &SwapParams,
    ) -> Result<SwapAndAccountMetas, AmmError> {
        if self.verify_exact_out_params {
            assert_eq!(params.swap_mode, SwapMode::ExactOut);
            let quote = self.quote(&QuoteParams {
                amount: params.out_amount,
                input_mint: params.source_mint,
                output_mint: params.destination_mint,
                swap_mode: SwapMode::ExactOut,
                fee_mode: Default::default(),
            })?;
            assert_eq!(params.in_amount, quote.in_amount);
            assert_eq!(params.out_amount, 1_000_000);
        }
        let direction = self.direction(params.source_mint, params.destination_mint)?;
        // Native Raydium account order, without a leading Jupiter adapter program meta.
        let authority = Pubkey::find_program_address(&[b"vault_and_lp_mint_auth_seed"], &PROGRAM).0;
        Ok(SwapAndAccountMetas {
            swap: Swap::Placeholder { data: None },
            account_metas: vec![
                AccountMeta::new_readonly(params.token_transfer_authority, true),
                AccountMeta::new_readonly(authority, false),
                AccountMeta::new_readonly(self.config, false),
                AccountMeta::new(self.key, false),
                AccountMeta::new(params.source_token_account, false),
                AccountMeta::new(params.destination_token_account, false),
                AccountMeta::new(self.vaults[direction], false),
                AccountMeta::new(self.vaults[1 - direction], false),
                AccountMeta::new_readonly(self.token_programs[direction], false),
                AccountMeta::new_readonly(self.token_programs[1 - direction], false),
                AccountMeta::new_readonly(params.source_mint, false),
                AccountMeta::new_readonly(params.destination_mint, false),
                AccountMeta::new(self.observation, false),
            ],
        })
    }
}

#[cfg(feature = "rpc")]
mod rpc_tests {
    use super::*;
    use base64::Engine;
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpListener, TcpStream},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, JoinHandle},
        time::Duration,
    };

    #[test]
    fn snapshot_discovery_uses_exact_out_mode_and_quoted_input() {
        let mut test = exact_out_test();
        test.params = Some(serde_json::json!({"verify_exact_out_params": true}));
        let snapshot = PoolSnapshot::load_dir(&test.fixtures_dir).unwrap();
        let keyed_account = KeyedAccount {
            key: POOL,
            account: snapshot.get(&POOL).unwrap().clone(),
            params: test.params.clone(),
        };
        let amm = RaydiumCpmm::from_keyed_account(&keyed_account, &AmmContext::default()).unwrap();
        let server = SnapshotServer::new(snapshot);
        let fetched = PoolSnapshot::from_rpc(&server.url(), &amm, &test.swaps).unwrap();
        assert!(fetched.get(&POOL).is_some());
        assert!(fetched.get(&USDC).is_some());
        assert!(fetched.get(&WSOL).is_some());
    }

    /// A loopback JSON-RPC server makes the snapshot path reproducible without a live RPC.
    struct SnapshotServer {
        address: SocketAddr,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl SnapshotServer {
        fn new(snapshot: PoolSnapshot) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = stop.clone();
            let worker = thread::spawn(move || {
                for stream in listener.incoming() {
                    if worker_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut stream = stream.unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    let body_start = loop {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                        if request.ends_with(b"\r\n\r\n") {
                            break request.len();
                        }
                    };
                    let headers = String::from_utf8_lossy(&request);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        })
                        .unwrap()
                        .1
                        .trim()
                        .parse()
                        .unwrap();
                    request.resize(body_start + length, 0);
                    stream.read_exact(&mut request[body_start..]).unwrap();
                    let body: serde_json::Value =
                        serde_json::from_slice(&request[body_start..]).unwrap();
                    assert_eq!(body["method"], "getMultipleAccounts");
                    let values: Vec<serde_json::Value> = body["params"][0].as_array().unwrap().iter()
                        .map(|key| {
                            let key: Pubkey = key.as_str().unwrap().parse().unwrap();
                            snapshot.get(&key).map_or(serde_json::Value::Null, |account| serde_json::json!({
                                "lamports": account.lamports,
                                "data": [base64::engine::general_purpose::STANDARD.encode(&account.data), "base64"],
                                "owner": account.owner.to_string(),
                                "executable": account.executable,
                                "rentEpoch": account.rent_epoch,
                            }))
                        }).collect();
                    let response = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {"context": {"slot": 1}, "value": values}}).to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
                }
            });
            Self {
                address,
                stop,
                worker: Some(worker),
            }
        }

        fn url(&self) -> String {
            format!("http://{}", self.address)
        }
    }

    impl Drop for SnapshotServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(self.address);
            self.worker.take().unwrap().join().unwrap();
        }
    }
}

impl RaydiumCpmm {
    fn direction(&self, input: Pubkey, output: Pubkey) -> Result<usize, AmmError> {
        if input == self.mints[0] && output == self.mints[1] {
            Ok(0)
        } else if input == self.mints[1] && output == self.mints[0] {
            Ok(1)
        } else {
            Err("invalid token pair".into())
        }
    }
}
