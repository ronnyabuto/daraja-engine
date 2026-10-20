//! Financial domain contracts. Persistence and trusted provider verification are separate boundaries.

pub mod amount;
pub mod identity;
pub mod inbound;
pub mod outbound;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(pub u64);
