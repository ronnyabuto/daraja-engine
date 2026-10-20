use serde::Serialize;

use crate::{Error, Timestamp, identity::Id};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// One hundredth of a Kenyan shilling. Provider whole-KES conversion must be exact.
    KesMinor,
    Sat,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Amount {
    value: u64,
    unit: Unit,
}

impl Amount {
    /// Nonnegative fixed KES decimal, at most two places, no sign or exponent.
    pub fn kes_decimal(s: &str) -> Result<Self, Error> {
        let mut parts = s.split('.');
        let whole = parts.next().ok_or(Error::InvalidAmount)?;
        let fraction = parts.next();
        let frac = fraction.unwrap_or("");
        if whole.is_empty()
            || !whole.bytes().all(|b| b.is_ascii_digit())
            || parts.next().is_some()
            || (fraction.is_some() && frac.is_empty())
            || frac.len() > 2
            || !frac.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(Error::InvalidAmount);
        }
        let major = whole.parse::<u64>().map_err(|_| Error::Overflow)?;
        let minor = if frac.is_empty() {
            0
        } else {
            frac.parse::<u64>().map_err(|_| Error::InvalidAmount)?
                * if frac.len() == 1 { 10 } else { 1 }
        };
        Ok(Self::new(
            major
                .checked_mul(100)
                .and_then(|n| n.checked_add(minor))
                .ok_or(Error::Overflow)?,
            Unit::KesMinor,
        ))
    }

    pub const fn new(value: u64, unit: Unit) -> Self {
        Self { value, unit }
    }

    pub const fn value(self) -> u64 {
        self.value
    }

    pub const fn unit(self) -> Unit {
        self.unit
    }

    pub fn checked_add(self, other: Self) -> Result<Self, Error> {
        self.same_unit(other)?;
        Ok(Self::new(
            self.value.checked_add(other.value).ok_or(Error::Overflow)?,
            self.unit,
        ))
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, Error> {
        self.same_unit(other)?;
        Ok(Self::new(
            self.value
                .checked_sub(other.value)
                .ok_or(Error::InvalidAmount)?,
            self.unit,
        ))
    }

    fn same_unit(self, other: Self) -> Result<(), Error> {
        if self.unit != other.unit {
            return Err(Error::CurrencyMismatch);
        }
        Ok(())
    }

    pub fn require_kes(self) -> Result<(), Error> {
        if self.unit != Unit::KesMinor {
            return Err(Error::CurrencyMismatch);
        }
        Ok(())
    }

    pub fn ceil_whole_kes(self) -> Result<Self, Error> {
        self.require_kes()?;
        let whole = self.value / 100 + u64::from(!self.value.is_multiple_of(100));
        Ok(Self::new(
            whole.checked_mul(100).ok_or(Error::Overflow)?,
            Unit::KesMinor,
        ))
    }
    pub fn whole_kes(self) -> Result<u64, Error> {
        self.require_kes()?;
        if !self.value.is_multiple_of(100) {
            return Err(Error::InvalidAmount);
        }
        Ok(self.value / 100)
    }
}

/// A frozen rational number of sat per KES minor unit, rounded down on issuance.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FxSnapshot {
    pub source: Id,
    pub numerator: u64,
    pub denominator: u64,
    pub expires_at: Timestamp,
}

impl FxSnapshot {
    pub fn validate_at(&self, now: Timestamp) -> Result<(), Error> {
        if self.numerator == 0 || self.denominator == 0 {
            return Err(Error::InvalidAmount);
        }
        if now >= self.expires_at {
            return Err(Error::FxExpired);
        }
        Ok(())
    }
    /// Debit KES value in sat, rounding up to avoid spending unbacked float.
    pub fn debit(&self, kes: Amount) -> Result<Amount, Error> {
        kes.require_kes()?;
        Ok(Amount::new(
            ratio(kes.value(), self.numerator, self.denominator, true)?,
            Unit::Sat,
        ))
    }
    /// Backing available from sat, rounded down to KES minor units for fee budgeting.
    pub fn value_kes(&self, sat: Amount) -> Result<Amount, Error> {
        if sat.unit() != Unit::Sat {
            return Err(Error::CurrencyMismatch);
        }
        Ok(Amount::new(
            ratio(sat.value(), self.denominator, self.numerator, false)?,
            Unit::KesMinor,
        ))
    }
    /// Payout from sat, rounding down to whole KES. Dust remains reserved.
    pub fn payout_kes(&self, sat: Amount) -> Result<Amount, Error> {
        let minor = self.value_kes(sat)?.value();
        Ok(Amount::new(minor / 100 * 100, Unit::KesMinor))
    }
    /// Collection backing for sat, rounded up to whole KES.
    pub fn backing_kes(&self, sat: Amount) -> Result<Amount, Error> {
        self.backing_minor(sat)?.ceil_whole_kes()
    }

    pub fn backing_minor(&self, sat: Amount) -> Result<Amount, Error> {
        if sat.unit() != Unit::Sat {
            return Err(Error::CurrencyMismatch);
        }
        Ok(Amount::new(
            ratio(sat.value(), self.denominator, self.numerator, true)?,
            Unit::KesMinor,
        ))
    }
    pub fn credit(&self, kes: Amount) -> Result<Amount, Error> {
        kes.require_kes()?;
        if self.numerator == 0 || self.denominator == 0 {
            return Err(Error::InvalidAmount);
        }
        let scaled = u128::from(kes.value) * u128::from(self.numerator);
        let value =
            u64::try_from(scaled / u128::from(self.denominator)).map_err(|_| Error::Overflow)?;
        Ok(Amount::new(value, Unit::Sat))
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct TariffSnapshot {
    pub version: Id,
    pub fee: Amount,
}

#[derive(serde::Deserialize)]
#[serde(remote = "Unit")]
enum StoredUnit {
    /// One hundredth of a Kenyan shilling. Provider whole-KES conversion must be exact.
    KesMinor,
    Sat,
}
impl crate::storage::Stored for Unit {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredUnit::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "Amount")]
struct StoredAmount {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    value: u64,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    unit: Unit,
}
impl crate::storage::Stored for Amount {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredAmount::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "FxSnapshot")]
struct StoredFxSnapshot {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    source: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    numerator: u64,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    denominator: u64,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    expires_at: Timestamp,
}
impl crate::storage::Stored for FxSnapshot {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredFxSnapshot::deserialize(d)
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "TariffSnapshot")]
struct StoredTariffSnapshot {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    version: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    fee: Amount,
}
impl crate::storage::Stored for TariffSnapshot {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredTariffSnapshot::deserialize(d)
    }
}

fn ratio(value: u64, numerator: u64, denominator: u64, ceil: bool) -> Result<u64, Error> {
    if numerator == 0 || denominator == 0 {
        return Err(Error::InvalidAmount);
    }
    let product = u128::from(value) * u128::from(numerator);
    let d = u128::from(denominator);
    u64::try_from(product / d + u128::from(ceil && product % d != 0)).map_err(|_| Error::Overflow)
}
