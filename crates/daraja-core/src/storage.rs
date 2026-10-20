//! Private decoding keeps database snapshots out of public request deserialization.
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;

pub(crate) trait Stored: Sized {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error>;
}
pub(crate) struct Value<T>(T);
impl<'de, T: Stored> Deserialize<'de> for Value<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(T::read(d)?))
    }
}
impl<T: Stored> Value<T> {
    pub(crate) fn deserialize_json(s: &str) -> Result<T, serde_json::Error> {
        Ok(serde_json::from_str::<Self>(s)?.0)
    }
}
pub(crate) fn deserialize<'de, T: Stored, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
    T::read(d)
}
macro_rules! primitive {
    ($($t:ty),*) => { $(impl Stored for $t {
        fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> { Deserialize::deserialize(d) }
    })* };
}
primitive!(String, u64, u8, [u8; 32]);
impl<T: Stored> Stored for Vec<T> {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Vec::<Value<T>>::deserialize(d)?
            .into_iter()
            .map(|v| v.0)
            .collect())
    }
}
impl<T: Stored> Stored for Option<T> {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Option::<Value<T>>::deserialize(d)?.map(|v| v.0))
    }
}
impl Stored for BTreeMap<String, String> {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Deserialize::deserialize(d)
    }
}
