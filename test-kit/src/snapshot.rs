//! Pool account snapshots: the accounts captured from a live pool, saved/loaded as
//! `<pubkey>.bin` fixture files.

use std::{
    collections::HashMap,
    fs::{File, create_dir_all},
    io,
    path::Path,
};

use jupiter_amm_interface::{AccountProvider, Amm};
use solana_account::{Account, ReadableAccount};
use solana_clock::Clock;
use solana_pubkey::Pubkey;

pub(crate) const CLOCK_SYSVAR_ID: Pubkey =
    solana_pubkey::pubkey!("SysvarC1ock11111111111111111111111111111111");

/// A set of on-chain accounts captured from a pool, enough to `update()` and quote the AMM
/// and to execute its swap in LiteSVM.
#[derive(Clone, Default)]
pub struct PoolSnapshot {
    pub(crate) accounts: HashMap<Pubkey, Account>,
}

impl PoolSnapshot {
    pub fn from_accounts(accounts: HashMap<Pubkey, Account>) -> Self {
        Self { accounts }
    }

    /// Load a snapshot from a directory of `<pubkey>.bin` bincode-serialized [`Account`]s
    /// (the layout produced by [`PoolSnapshot::save_dir`]).
    pub fn load_dir(dir: impl AsRef<Path>) -> io::Result<Self> {
        let mut accounts = HashMap::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("bin") {
                continue;
            }
            let Some(pubkey) = path
                .file_stem()
                .and_then(|stem| stem.to_str()?.parse::<Pubkey>().ok())
            else {
                continue;
            };
            let account = bincode::deserialize_from(File::open(&path)?)
                .map_err(|e| io::Error::other(e.to_string()))?;
            accounts.insert(pubkey, account);
        }
        Ok(Self { accounts })
    }

    pub fn save_dir(&self, dir: impl AsRef<Path>) -> io::Result<()> {
        create_dir_all(&dir)?;
        for (pubkey, account) in &self.accounts {
            let file = File::create(dir.as_ref().join(format!("{pubkey}.bin")))?;
            bincode::serialize_into(file, account).map_err(|e| io::Error::other(e.to_string()))?;
        }
        Ok(())
    }

    pub fn get(&self, pubkey: &Pubkey) -> Option<&Account> {
        self.accounts.get(pubkey)
    }

    /// The clock captured in the snapshot, if present. LiteSVM picks the clock up from the
    /// snapshotted sysvar account itself; this accessor serves the `ClockRef` handed to
    /// `Amm::from_keyed_account`.
    pub fn clock(&self) -> Option<Clock> {
        bincode::deserialize(self.accounts.get(&CLOCK_SYSVAR_ID)?.data()).ok()
    }
}

/// `AccountProvider` view over a snapshot, for `Amm::update`.
pub(crate) struct SnapshotProvider<'a>(pub(crate) &'a HashMap<Pubkey, Account>);

impl<'a> AccountProvider for SnapshotProvider<'a> {
    fn get(&self, pubkey: &Pubkey) -> Option<impl ReadableAccount + use<'_, 'a>> {
        self.0.get(pubkey).cloned()
    }
}

/// Hydrate an AMM from a snapshot (a few `update` passes to settle any two-phase state).
pub(crate) fn update_amm_from_snapshot<A: Amm>(amm: &mut A, snapshot: &PoolSnapshot) {
    for _ in 0..3 {
        amm.update(SnapshotProvider(&snapshot.accounts))
            .expect("Amm::update from snapshot");
    }
}
