//! Helpers shared by the unit tests.

/// The value as JSON; fails on a repeated key.
///
/// Every serialized result type must have unique keys: a tagged enum's
/// `kind` tag must not meet a field called `kind`, or a JSON reader keeps
/// only one of them.
#[cfg(feature = "serde")]
pub(crate) fn unique(value: &impl serde::Serialize) -> String {
    let json = serde_json::to_string(value).unwrap();
    if let Err(e) = serde_json::from_str::<Unique>(&json) {
        panic!("{json}: {e}");
    }
    json
}

/// Any JSON value; deserializing fails on a repeated key.
#[cfg(feature = "serde")]
struct Unique;

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Unique {
    fn deserialize<D: serde::de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(UniqueVisitor)
    }
}

#[cfg(feature = "serde")]
struct UniqueVisitor;

#[cfg(feature = "serde")]
impl<'de> serde::de::Visitor<'de> for UniqueVisitor {
    type Value = Unique;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JSON")
    }
    fn visit_bool<E>(self, _: bool) -> Result<Unique, E> {
        Ok(Unique)
    }
    fn visit_i64<E>(self, _: i64) -> Result<Unique, E> {
        Ok(Unique)
    }
    fn visit_u64<E>(self, _: u64) -> Result<Unique, E> {
        Ok(Unique)
    }
    fn visit_f64<E>(self, _: f64) -> Result<Unique, E> {
        Ok(Unique)
    }
    fn visit_str<E>(self, _: &str) -> Result<Unique, E> {
        Ok(Unique)
    }
    fn visit_unit<E>(self) -> Result<Unique, E> {
        Ok(Unique)
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Unique, A::Error> {
        while seq.next_element::<Unique>()?.is_some() {}
        Ok(Unique)
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
        let mut keys = std::collections::HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!("duplicate key {key:?}")));
            }
            map.next_value::<Unique>()?;
        }
        Ok(Unique)
    }
}
