use crate::{Error, Timestamp, identity::Id};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// One hundredth of a Kenyan shilling. Provider whole-KES conversion must be exact.
    KesMinor,
    Sat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Amount {
    value: u64,
    unit: Unit,
}

impl Amount {
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

    pub fn whole_kes(self) -> Result<u64, Error> {
        self.require_kes()?;
        if !self.value.is_multiple_of(100) {
            return Err(Error::InvalidAmount);
        }
        Ok(self.value / 100)
    }
}

/// A frozen rational number of sat per KES minor unit, rounded down on issuance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FxSnapshot {
    pub source: Id,
    pub numerator: u64,
    pub denominator: u64,
    pub expires_at: Timestamp,
}

impl FxSnapshot {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TariffSnapshot {
    pub version: Id,
    pub fee: Amount,
}
