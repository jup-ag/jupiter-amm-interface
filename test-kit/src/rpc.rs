//! RPC snapshotter (feature = "rpc"): a minimal JSON-RPC `getMultipleAccounts` client.
//! The kit only ever batch-reads accounts, so a blocking `ureq` call keeps it sync and
//! avoids pulling an async RPC client stack (tokio, reqwest) into a test kit.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use jupiter_amm_interface::{Amm, AmmContext, ClockRef, KeyedAccount, SwapMode, SwapParams};
use solana_account::Account;
use solana_clock::Clock;
use solana_pubkey::Pubkey;

use crate::parity::{PoolTest, SwapTest};
use crate::snapshot::{CLOCK_SYSVAR_ID, PoolSnapshot, SnapshotProvider};
use crate::svm::{program_names, program_path};

/// Error from the RPC snapshotter.
#[derive(Debug)]
pub struct RpcError(pub String);

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rpc snapshot error: {}", self.0)
    }
}
impl std::error::Error for RpcError {}

impl PoolSnapshot {
    /// Snapshot everything needed to quote and swap `amm` from a live RPC endpoint.
    ///
    /// Runs a few fetch+update rounds so AMMs whose `update` reveals further accounts
    /// settle, then discovers the remaining accounts from `swaps`' account metas
    /// (vaults, authorities, fee accounts, oracles, …), the reserve mints, and the
    /// standard sysvars. Executables are dropped (programs are supplied to the harness
    /// as `.so` separately).
    ///
    /// `amm` need not be pre-`update`d: a clone is hydrated internally.
    pub fn from_rpc<A: Amm>(rpc_url: &str, amm: &A, swaps: &[SwapTest]) -> Result<Self, RpcError> {
        let mut addrs: HashSet<Pubkey> = HashSet::new();
        addrs.insert(amm.key());
        addrs.extend(amm.get_reserve_mints());

        // `get_accounts_to_update` can grow as state loads (e.g. tick arrays discovered
        // once the pool account is read), so re-ask after every update round.
        let mut probe = amm.clone();
        let mut accounts: HashMap<Pubkey, Account> = HashMap::new();
        for _ in 0..3 {
            let round = probe.get_accounts_to_update();
            addrs.extend(round.iter().copied());
            accounts.extend(fetch_map(rpc_url, &round)?);
            let _ = probe.update(SnapshotProvider(&accounts));
        }

        let placeholder = Pubkey::new_unique();
        for swap in swaps {
            let sam = probe
                .get_swap_and_account_metas(&SwapParams {
                    swap_mode: SwapMode::ExactIn,
                    in_amount: swap.amount,
                    out_amount: swap.amount,
                    source_mint: swap.input_mint,
                    destination_mint: swap.output_mint,
                    source_token_account: placeholder,
                    destination_token_account: placeholder,
                    token_transfer_authority: placeholder,
                    user: placeholder,
                    payer: placeholder,
                    quote_mint_to_referrer: None,
                    jupiter_program_id: &placeholder,
                    missing_dynamic_accounts_as_default: false,
                })
                .map_err(|e| RpcError(format!("get_swap_and_account_metas: {e}")))?;
            addrs.extend(sam.account_metas.into_iter().map(|m| m.pubkey));
        }

        // Sysvars the on-chain program may read implicitly (syscalls, not account metas),
        // so they never appear in `get_accounts_to_update` or the swap's metas.
        addrs.extend([
            CLOCK_SYSVAR_ID,
            solana_pubkey::pubkey!("SysvarLastRestartS1ot1111111111111111111111"),
            solana_pubkey::pubkey!("SysvarStakeHistory1111111111111111111111111"),
        ]);
        addrs.remove(&placeholder);

        let mut accounts = fetch_map(rpc_url, &addrs.iter().copied().collect::<Vec<_>>())?;
        // Programs are loaded into the harness as `.so`; don't snapshot executables.
        accounts.retain(|_, a| !a.executable);
        Ok(Self { accounts })
    }
}

impl PoolTest {
    /// Regenerate this test's fixtures from a live RPC endpoint: fetch the pool account,
    /// build `A` from it, snapshot the accounts ([`PoolSnapshot::from_rpc`]) to the test's
    /// fixtures dir, and dump each *missing* program's ELF (what `solana program dump`
    /// produces) to `tests/fixtures/<name>.so`. Existing `.so` files are left alone, so a
    /// locally built program is never overwritten by the deployed one.
    ///
    /// [`assert_pool_parity`](crate::assert_pool_parity) calls this itself when `RPC` is
    /// set and the fixtures are missing (or `REFRESH` is set); it's public for custom
    /// flows.
    pub fn regenerate_fixtures<A: Amm>(&self, rpc_url: &str) -> Result<(), RpcError> {
        let account = fetch_account(rpc_url, &self.pool)?
            .ok_or_else(|| RpcError(format!("pool {} not found", self.pool)))?;
        let clock: Clock = fetch_account(rpc_url, &CLOCK_SYSVAR_ID)?
            .and_then(|account| bincode::deserialize(&account.data).ok())
            .unwrap_or_default();
        let keyed_account = KeyedAccount {
            key: self.pool,
            account,
            params: self.params.clone(),
        };
        let amm_context = AmmContext {
            clock_ref: ClockRef::from(clock),
        };
        let amm = A::from_keyed_account(&keyed_account, &amm_context)
            .map_err(|e| RpcError(format!("from_keyed_account: {e}")))?;

        let snapshot = PoolSnapshot::from_rpc(rpc_url, &amm, &self.swaps)?;
        // Clear any previous snapshot so stale account files don't linger.
        if self.fixtures_dir.exists() {
            std::fs::remove_dir_all(&self.fixtures_dir)
                .map_err(|e| RpcError(format!("clear fixtures dir: {e}")))?;
        }
        snapshot
            .save_dir(&self.fixtures_dir)
            .map_err(|e| RpcError(format!("save_dir: {e}")))?;
        for (program_id, name) in program_names(&amm) {
            let path = program_path(&name);
            // Only dump missing programs — an existing `.so` may be a locally built
            // version that isn't (or differs from what's) deployed. Delete it to re-dump.
            if path.exists() {
                continue;
            }
            let elf = dump_program(rpc_url, &program_id)?;
            std::fs::write(&path, elf).map_err(|e| RpcError(format!("write {name}.so: {e}")))?;
        }
        Ok(())
    }
}

const BPF_LOADER_UPGRADEABLE: Pubkey =
    solana_pubkey::pubkey!("BPFLoaderUpgradeab1e11111111111111111111111");

/// Fetch a program's ELF — what `solana program dump` produces.
fn dump_program(rpc_url: &str, program_id: &Pubkey) -> Result<Vec<u8>, RpcError> {
    let account = fetch_account(rpc_url, program_id)?
        .ok_or_else(|| RpcError(format!("program {program_id} not found")))?;
    if account.owner != BPF_LOADER_UPGRADEABLE {
        // Non-upgradeable loaders keep the ELF in the program account itself.
        return Ok(account.data);
    }
    // Upgradeable loader: the program account points at the programdata account, which
    // holds the ELF after 45 bytes of metadata (enum tag, deploy slot, upgrade authority).
    let programdata_address = account
        .data
        .get(4..36)
        .map(|bytes| Pubkey::try_from(bytes).unwrap())
        .ok_or_else(|| RpcError(format!("malformed program account {program_id}")))?;
    let programdata = fetch_account(rpc_url, &programdata_address)?
        .ok_or_else(|| RpcError(format!("programdata {programdata_address} not found")))?;
    programdata
        .data
        .get(45..)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| RpcError(format!("programdata {programdata_address} too short")))
}

/// Fetch a single account (`None` if it doesn't exist).
pub fn fetch_account(rpc_url: &str, address: &Pubkey) -> Result<Option<Account>, RpcError> {
    Ok(fetch_map(rpc_url, &[*address])?.remove(address))
}

/// Fetch multiple accounts; missing/`null` entries are omitted from the map.
fn fetch_map(rpc_url: &str, addrs: &[Pubkey]) -> Result<HashMap<Pubkey, Account>, RpcError> {
    let mut out = HashMap::new();
    for chunk in addrs.chunks(100) {
        let keys: Vec<String> = chunk.iter().map(|p| p.to_string()).collect();
        let resp = call(
            rpc_url,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getMultipleAccounts",
                "params": [keys, {"encoding": "base64"}],
            }),
        )?;
        let value = resp
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| RpcError(format!("unexpected getMultipleAccounts response: {resp}")))?;
        for (pubkey, entry) in chunk.iter().zip(value) {
            if let Some(account) = parse_account(entry) {
                out.insert(*pubkey, account);
            }
        }
    }
    Ok(out)
}

fn call(rpc_url: &str, body: serde_json::Value) -> Result<serde_json::Value, RpcError> {
    let resp: serde_json::Value = ureq::post(rpc_url)
        .send_json(body)
        .map_err(|e| RpcError(e.to_string()))?
        .into_json()
        .map_err(|e| RpcError(e.to_string()))?;
    if let Some(err) = resp.get("error") {
        return Err(RpcError(format!("rpc returned error: {err}")));
    }
    Ok(resp)
}

fn parse_account(entry: &serde_json::Value) -> Option<Account> {
    if entry.is_null() {
        return None;
    }
    let data_b64 = entry.get("data")?.get(0)?.as_str()?;
    Some(Account {
        lamports: entry.get("lamports")?.as_u64()?,
        data: base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .ok()?,
        owner: entry.get("owner")?.as_str()?.parse().ok()?,
        executable: entry
            .get("executable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        rent_epoch: entry.get("rentEpoch").and_then(|v| v.as_u64()).unwrap_or(0),
    })
}
