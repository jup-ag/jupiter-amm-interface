//! # jupiter-amm-test-kit
//!
//! A self-verification harness for Jupiter AMM integrations.
//! Integrators should use this to prove that their `Amm::quote()`
//! exactly matches their on-chain program math.
//!
//! 1. Load a [`PoolSnapshot`].
//! 2. Run the SDK's pure `Amm::quote`.
//! 3. Execute the AMM program's swap instruction in LiteSVM.
//! 4. Assert the realized token delta equals the quote.
//!
//! Cover as many pool types and swap paths as possible —
//! for example: limit orders, dynamic fees, different fee modes, etc.
//!
//! Besides the `Amm` impl itself, the only thing to write is a small closure that encodes
//! the program's native swap instruction data (discriminator + args).
//! Everything else is derived from the trait.
//!
//! Note: only ExactIn swaps are asserted, and programs that reject top-level invocations
//! (expecting a CPI from an approved integrator) are not yet supported. See the README's
//! Limitations section for the full list.

mod parity;
#[cfg(feature = "rpc")]
mod rpc;
mod snapshot;
mod svm;

pub use parity::{PoolTest, SwapTest, assert_pool_parity};
#[cfg(feature = "rpc")]
pub use rpc::{RpcError, fetch_account};
pub use snapshot::PoolSnapshot;
