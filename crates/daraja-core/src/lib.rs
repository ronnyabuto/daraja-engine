//! Financial domain contracts. Persistence and trusted provider verification are separate boundaries.

pub mod amount;
pub mod auth;
pub mod checkout;
pub mod collection;
pub mod crypto;
pub mod identity;
pub mod inbound;
pub mod outbound;
pub mod payout;
pub mod reconcile;
pub mod repository;
mod storage;
pub mod transport;
pub mod webhook;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Overflow,
    CurrencyMismatch,
    InvalidAmount,
    InvalidIdentity,
    IdempotencyConflict,
    DuplicateIdentity,
    DuplicateReceipt,
    InvalidTransition,
    EvidenceMismatch,
    NotFound,
    QuoteExpired,
    FxExpired,
    SettlementHeld,
    AmountMismatch,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

/// UTC seconds supplied by the caller; domain transitions never read the wall clock.
#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub u64);

impl storage::Stored for Timestamp {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(serde::Deserialize::deserialize(d)?))
    }
}
