pub mod field_as_string;
pub mod option_field_as_string;

fn parse_value<T, E>(value: &str) -> Result<T, E>
where
    T: std::str::FromStr,
    E: ::serde::de::Error,
    <T as std::str::FromStr>::Err: std::fmt::Debug,
{
    value
        .parse()
        .map_err(|error| E::custom(format!("Parse error: {error:?}")))
}
