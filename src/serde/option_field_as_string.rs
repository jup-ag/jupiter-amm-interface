use {
    super::{field_as_string, parse_value},
    ::serde::{Deserialize, Deserializer, Serializer},
    std::str::FromStr,
};

pub fn serialize<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    T: ToString,
    S: Serializer,
{
    if let Some(value) = value {
        field_as_string::serialize(value, serializer)
    } else {
        serializer.serialize_none()
    }
}

pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: FromStr,
    D: Deserializer<'de>,
    <T as FromStr>::Err: std::fmt::Debug,
{
    match Option::<&str>::deserialize(deserializer)? {
        Some(s) => parse_value(s).map(Some),
        None => Ok(None),
    }
}
