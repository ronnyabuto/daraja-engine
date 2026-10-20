use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::{Error, amount::Amount};

/// Opaque identity; Debug deliberately omits values which may contain customer data.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Id(String);

impl Id {
    pub fn new(value: impl Into<String>) -> Result<Self, Error> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Id([redacted])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PayloadHash([u8; 32]);

impl PayloadHash {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// Provider receipt identities use one uppercase ASCII alphanumeric representation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReceiptId(Id);

impl ReceiptId {
    pub fn new(value: &str) -> Result<Self, Error> {
        if !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self(Id::new(value.to_ascii_uppercase())?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Canonical v1 domain payload: ordered, length-prefixed UTF-8 fields, big-endian
/// integers, explicit unit. Transport adapters must persist exact wire bytes separately.
#[derive(Clone, PartialEq, Eq)]
pub struct RequestPayload {
    pub operation: Id,
    pub quote_id: Id,
    pub destination: Id,
    pub amount: Amount,
    pub tariff_version: Id,
    pub options: BTreeMap<String, String>,
}

impl std::fmt::Debug for RequestPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RequestPayload([redacted])")
    }
}

impl RequestPayload {
    pub fn hash(&self) -> PayloadHash {
        let mut digest = Sha256::new();
        digest.update(b"daraja.request.v1\0");
        let mut field = |value: &str| {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        };
        for value in [
            &self.operation,
            &self.quote_id,
            &self.destination,
            &self.tariff_version,
        ] {
            field(value.as_str());
        }
        field(match self.amount.unit() {
            crate::amount::Unit::KesMinor => "kes_minor",
            crate::amount::Unit::Sat => "sat",
        });
        digest.update(self.amount.value().to_be_bytes());
        digest.update((self.options.len() as u64).to_be_bytes());
        for (key, value) in &self.options {
            for part in [key, value] {
                digest.update((part.len() as u64).to_be_bytes());
                digest.update(part.as_bytes());
            }
        }
        PayloadHash(digest.finalize().into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestKey {
    pub caller: Id,
    pub operation: Id,
    pub key: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestBinding {
    pub payload_hash: PayloadHash,
    pub operation_id: Id,
}

#[derive(Default)]
pub struct RequestRegistry(BTreeMap<RequestKey, RequestBinding>);

impl RequestRegistry {
    /// Replays return the original operation identity. Different payloads never replace it.
    pub fn bind(
        &mut self,
        key: RequestKey,
        payload: &RequestPayload,
        operation_id: Id,
    ) -> Result<RequestBinding, Error> {
        if key.operation != payload.operation {
            return Err(Error::EvidenceMismatch);
        }
        let hash = payload.hash();
        if let Some(existing) = self.0.get(&key) {
            if existing.payload_hash != hash {
                return Err(Error::IdempotencyConflict);
            }
            return Ok(existing.clone());
        }
        let binding = RequestBinding {
            payload_hash: hash,
            operation_id,
        };
        self.0.insert(key, binding.clone());
        Ok(binding)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correlation {
    pub caller_request: Id,
    pub originator_conversation: Id,
    pub provider_originator: Option<Id>,
    pub provider_conversation: Option<Id>,
}

impl Correlation {
    pub fn acknowledge(&mut self, originator: Id, conversation: Id) -> Result<(), Error> {
        if self
            .provider_originator
            .as_ref()
            .is_some_and(|id| id != &originator)
            || self
                .provider_conversation
                .as_ref()
                .is_some_and(|id| id != &conversation)
        {
            return Err(Error::EvidenceMismatch);
        }
        self.provider_originator = Some(originator);
        self.provider_conversation = Some(conversation);
        Ok(())
    }
}
