//! Reusable SSH host catalog and foreground operations. No global runtime or CLI dependency.
#![cfg(unix)]
pub mod credentials;
pub mod error;
pub mod importer;
pub mod model;
pub mod process;
pub mod reach;
pub mod ssh;
pub mod store;
pub mod transfer;
pub mod trust;
pub use credentials::{CredentialStore, SystemCredentials};
pub use error::{Error, Result};
pub use model::*;
pub use ssh::{ExecutionOptions, OpenSsh};
pub use store::Store;
