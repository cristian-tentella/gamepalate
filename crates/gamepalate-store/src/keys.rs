// An appid or a tagid used as a TOML key: digits without leading zeros.
pub(crate) fn parse_id_key(key: &str) -> Option<u32> {
    let well_formed = !key.starts_with('0') && key.chars().all(|c| c.is_ascii_digit());
    if well_formed { key.parse().ok() } else { None }
}
