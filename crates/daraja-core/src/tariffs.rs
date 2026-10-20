//! Operator-supplied corporate tariffs. There are no production tariff defaults.
use crate::{
    Error, Timestamp,
    amount::{Amount, TariffSnapshot},
    identity::Id,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rail {
    B2c,
    B2b,
}

/// Whole-KES principal interval [min_kes, max_kes], both inclusive.
#[derive(Clone, Debug)]
pub struct Band {
    pub min_kes: u64,
    pub max_kes: u64,
    pub fee: Amount,
}

/// UTC effective interval [effective_from, effective_until), end exclusive.
#[derive(Clone, Debug)]
pub struct Schedule {
    pub version: Id,
    pub effective_from: Timestamp,
    pub effective_until: Timestamp,
    pub bands: Vec<Band>,
}
impl Schedule {
    fn validate(&self) -> Result<(), Error> {
        if self.effective_from >= self.effective_until || self.bands.is_empty() {
            return Err(Error::InvalidAmount);
        }
        let mut next = self.bands[0].min_kes;
        if next == 0 {
            return Err(Error::InvalidAmount);
        }
        for b in &self.bands {
            b.fee.require_kes()?;
            if b.min_kes != next || b.max_kes < b.min_kes {
                return Err(Error::InvalidAmount);
            }
            next = b.max_kes.checked_add(1).ok_or(Error::Overflow)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Tariffs {
    b2c: Vec<Schedule>,
    b2b: Vec<Schedule>,
}
impl Tariffs {
    pub fn new(b2c: Vec<Schedule>, b2b: Vec<Schedule>) -> Result<Self, Error> {
        for schedules in [&b2c, &b2b] {
            if schedules.is_empty() {
                return Err(Error::InvalidAmount);
            }
            for (i, s) in schedules.iter().enumerate() {
                s.validate()?;
                if i > 0 && schedules[i - 1].effective_until != s.effective_from {
                    return Err(Error::InvalidAmount);
                }
                if schedules[..i].iter().any(|old| old.version == s.version) {
                    return Err(Error::DuplicateIdentity);
                }
            }
        }
        Ok(Self { b2c, b2b })
    }
    pub fn quote(
        &self,
        rail: Rail,
        principal: Amount,
        now: Timestamp,
    ) -> Result<TariffSnapshot, Error> {
        let n = principal.whole_kes()?;
        let s = self.schedule(rail, now)?;
        let b = s
            .bands
            .iter()
            .find(|b| b.min_kes <= n && n <= b.max_kes)
            .ok_or(Error::InvalidAmount)?;
        Ok(TariffSnapshot {
            version: s.version.clone(),
            fee: b.fee,
        })
    }
    /// Largest supported whole-KES principal whose exact fee fits the budget.
    /// Evaluate every band: operator schedules need not have monotone fees.
    pub fn principal_for_budget(
        &self,
        rail: Rail,
        budget: Amount,
        now: Timestamp,
    ) -> Result<Amount, Error> {
        budget.require_kes()?;
        let schedule = self.schedule(rail, now)?;
        let mut best = 0;
        for band in &schedule.bands {
            if let Some(net) = budget.value().checked_sub(band.fee.value()) {
                let candidate = (net / 100).min(band.max_kes);
                if candidate >= band.min_kes {
                    best = best.max(candidate);
                }
            }
        }
        if best == 0 {
            return Err(Error::InvalidAmount);
        }
        Ok(Amount::new(
            best.checked_mul(100).ok_or(Error::Overflow)?,
            crate::amount::Unit::KesMinor,
        ))
    }
    fn schedule(&self, rail: Rail, now: Timestamp) -> Result<&Schedule, Error> {
        let schedules = match rail {
            Rail::B2c => &self.b2c,
            Rail::B2b => &self.b2b,
        };
        schedules
            .iter()
            .find(|s| s.effective_from <= now && now < s.effective_until)
            .ok_or(Error::QuoteExpired)
    }
    pub fn effective_from(&self, rail: Rail, now: Timestamp) -> Result<Timestamp, Error> {
        Ok(self.schedule(rail, now)?.effective_from)
    }
    pub fn effective_until(&self, rail: Rail, now: Timestamp) -> Result<Timestamp, Error> {
        Ok(self.schedule(rail, now)?.effective_until)
    }
}
