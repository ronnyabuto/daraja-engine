//! Immutable outgoing valuation, independent of provider dispatch and CDK wire adapters.
use crate::{
    Error, Timestamp,
    amount::{Amount, FxSnapshot, TariffSnapshot, Unit},
    identity::Id,
    tariffs::{Rail, Tariffs},
};
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PaymentQuote {
    id: Id,
    destination: Id,
    principal: Amount,
    tariff: TariffSnapshot,
    tariff_effective_from: Timestamp,
    tariff_effective_until: Timestamp,
    fx: Option<FxSnapshot>,
    wallet_principal: Amount,
    wallet_reserved: Amount,
    expires_at: Timestamp,
}
impl PaymentQuote {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Id,
        destination: &str,
        principal: Amount,
        unit: Unit,
        tariffs: &Tariffs,
        fx: Option<FxSnapshot>,
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Result<Self, Error> {
        let parsed =
            crate::payout::Destination::parse(destination).map_err(|_| Error::InvalidIdentity)?;
        let (rail, destination) = match parsed {
            crate::payout::Destination::Phone(p) => (Rail::B2c, Id::new(p.expose())?),
            crate::payout::Destination::Till(_) | crate::payout::Destination::PayBill { .. } => {
                (Rail::B2b, Id::new(destination)?)
            }
        };
        let tariff = tariffs.quote(rail, principal, now)?;
        let mut expires_at = expires_at.min(tariffs.effective_until(rail, now)?);
        if let Some(f) = &fx {
            f.validate_at(now)?;
            if f.expires_at.0 > now.0.checked_add(300).ok_or(Error::Overflow)? {
                return Err(Error::InvalidAmount);
            }
            expires_at = expires_at.min(f.expires_at);
        }
        if now >= expires_at {
            return Err(Error::QuoteExpired);
        }
        let reserved = principal.checked_add(tariff.fee)?;
        let convert = |a| match (unit, &fx) {
            (Unit::KesMinor, None) => Ok(a),
            (Unit::Sat, Some(f)) => f.debit(a),
            _ => Err(Error::CurrencyMismatch),
        };
        let wallet_principal = convert(principal)?;
        let wallet_reserved = convert(reserved)?;
        Ok(Self {
            id,
            destination,
            principal,
            tariff,
            tariff_effective_from: tariffs.effective_from(rail, now)?,
            tariff_effective_until: tariffs.effective_until(rail, now)?,
            fx,
            wallet_principal,
            wallet_reserved,
            expires_at,
        })
    }
    pub fn from_sat_budget(
        id: Id,
        destination: &str,
        budget: Amount,
        tariffs: &Tariffs,
        fx: FxSnapshot,
        now: Timestamp,
        expires_at: Timestamp,
    ) -> Result<Self, Error> {
        fx.validate_at(now)?;
        let parsed =
            crate::payout::Destination::parse(destination).map_err(|_| Error::InvalidIdentity)?;
        let rail = match parsed {
            crate::payout::Destination::Phone(_) => Rail::B2c,
            _ => Rail::B2b,
        };
        let principal = tariffs.principal_for_budget(rail, fx.value_kes(budget)?, now)?;
        let quote = Self::new(
            id,
            destination,
            principal,
            Unit::Sat,
            tariffs,
            Some(fx),
            now,
            expires_at,
        )?;
        if quote.wallet_reserved.value() > budget.value() {
            return Err(Error::InvalidAmount);
        }
        Ok(quote)
    }
    pub fn id(&self) -> &Id {
        &self.id
    }
    pub fn destination(&self) -> &Id {
        &self.destination
    }
    pub fn principal(&self) -> Amount {
        self.principal
    }
    pub fn tariff(&self) -> &TariffSnapshot {
        &self.tariff
    }
    pub fn fx(&self) -> Option<&FxSnapshot> {
        self.fx.as_ref()
    }
    pub fn expires_at(&self) -> Timestamp {
        self.expires_at
    }
    pub fn wallet_principal(&self) -> Amount {
        self.wallet_principal
    }
    pub fn wallet_reserved(&self) -> Amount {
        self.wallet_reserved
    }
    /// Local CDK mapping: `kes` is whole KES; `sat` is one sat. Round charges up.
    pub fn cdk_principal(&self) -> Result<u64, Error> {
        match self.wallet_principal.unit() {
            Unit::KesMinor => self.wallet_principal.whole_kes(),
            Unit::Sat => Ok(self.wallet_principal.value()),
        }
    }
    pub fn cdk_fee_reserve(&self) -> Result<u64, Error> {
        self.cdk_total_spent(None)?
            .checked_sub(self.cdk_principal()?)
            .ok_or(Error::InvalidAmount)
    }
    pub fn cdk_total_spent(&self, actual: Option<Amount>) -> Result<u64, Error> {
        let total = self.total_spent(actual)?;
        Ok(match total.unit() {
            Unit::Sat => total.value(),
            Unit::KesMinor => total.value() / 100 + u64::from(!total.value().is_multiple_of(100)),
        })
    }
    pub fn fee_reserve(&self) -> Result<Amount, Error> {
        self.wallet_reserved.checked_sub(self.wallet_principal)
    }
    pub fn validate_dispatch(&self, now: Timestamp) -> Result<(), Error> {
        if self.principal.whole_kes()? == 0
            || self.tariff_effective_from >= self.tariff_effective_until
            || now < self.tariff_effective_from
            || self.expires_at > self.tariff_effective_until
        {
            return Err(Error::InvalidAmount);
        }
        let total = self.principal.checked_add(self.tariff.fee)?;
        let convert = |a| match (self.wallet_principal.unit(), &self.fx) {
            (Unit::KesMinor, None) => Ok(a),
            (Unit::Sat, Some(fx)) => fx.debit(a),
            _ => Err(Error::CurrencyMismatch),
        };
        if convert(self.principal)? != self.wallet_principal
            || convert(total)? != self.wallet_reserved
        {
            return Err(Error::AmountMismatch);
        }
        if now >= self.expires_at {
            return Err(Error::QuoteExpired);
        }
        if let Some(fx) = &self.fx {
            fx.validate_at(now)?;
        }
        Ok(())
    }
    /// Internal total includes principal and fee; `cdk_total_spent` maps to wire units.
    /// Missing fee evidence consumes the full reserve; excess requires reconciliation.
    pub fn total_spent(&self, actual_fee: Option<Amount>) -> Result<Amount, Error> {
        let Some(fee) = actual_fee else {
            return Ok(self.wallet_reserved);
        };
        fee.require_kes()?;
        if fee.value() > self.tariff.fee.value() {
            return Err(Error::SettlementHeld);
        }
        let total = self.principal.checked_add(fee)?;
        match &self.fx {
            Some(fx) => fx.debit(total),
            None => Ok(total),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(remote = "PaymentQuote")]
struct StoredPaymentQuote {
    #[serde(deserialize_with = "crate::storage::deserialize")]
    id: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    destination: Id,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    principal: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    tariff: TariffSnapshot,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    tariff_effective_from: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    tariff_effective_until: Timestamp,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    fx: Option<FxSnapshot>,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    wallet_principal: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    wallet_reserved: Amount,
    #[serde(deserialize_with = "crate::storage::deserialize")]
    expires_at: Timestamp,
}
impl crate::storage::Stored for PaymentQuote {
    fn read<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        StoredPaymentQuote::deserialize(d)
    }
}
