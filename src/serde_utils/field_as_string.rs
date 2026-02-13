use {
    super::parse_value,
    serde::{Deserialize, Deserializer, Serialize, Serializer},
    std::str::FromStr,
};

pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: ToString,
    S: Serializer,
{
    value.to_string().serialize(serializer)
}

pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
    T: FromStr,
    D: Deserializer<'de>,
    <T as FromStr>::Err: std::fmt::Debug,
{
    <&str>::deserialize(deserializer).and_then(parse_value)
}
