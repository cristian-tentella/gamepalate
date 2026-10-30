use gamepalate_core::timestamp::Timestamp;
use jiff::tz::TimeZone;

pub fn integer(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn decimal(x: f64) -> String {
    let text = format!("{:.2}", x.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, "00"));
    let sign = if x < 0.0 && text != "0.00" { "-" } else { "" };
    format!("{sign}{}.{fraction}", integer(whole.parse().unwrap_or(0)))
}

pub fn parse_number(text: &str) -> Result<f64, String> {
    text.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("\"{text}\" is not a number"))
}

pub fn parse_price(text: &str) -> Result<u32, String> {
    let error = || format!("\"{text}\" is not a price, for example 9.99 or 9,99");
    let text = text.trim().replace(',', ".");
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    if whole.is_empty() && fraction.is_empty()
        || fraction.len() > 2
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return Err(error());
    }
    let whole: u32 = if whole.is_empty() {
        0
    } else {
        whole.parse().map_err(|_| error())?
    };
    let fraction: u32 = format!("{fraction:0<2}").parse().map_err(|_| error())?;
    whole
        .checked_mul(100)
        .and_then(|cents| cents.checked_add(fraction))
        .ok_or_else(error)
}

pub fn multiplier(x: f64) -> String {
    let text = format!("{x:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("×{text}")
}

pub fn date(timestamp: Timestamp) -> String {
    local(timestamp, "%Y-%m-%d")
}

pub fn date_time(timestamp: Timestamp) -> String {
    local(timestamp, "%Y-%m-%d %H:%M")
}

fn local(timestamp: Timestamp, pattern: &str) -> String {
    match jiff::Timestamp::from_millisecond(timestamp.0) {
        Ok(time) => time
            .to_zoned(TimeZone::system())
            .strftime(pattern)
            .to_string(),
        Err(_) => format!("{} ms", timestamp.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_price_takes_a_dot_or_a_comma() {
        assert_eq!(parse_price("9.99"), Ok(999));
        assert_eq!(parse_price("9,99"), Ok(999));
        assert_eq!(parse_price("15"), Ok(1500));
        assert_eq!(parse_price("0,5"), Ok(50));
    }

    #[test]
    fn a_price_that_is_not_a_number_says_so_without_naming_a_currency() {
        let error = parse_price("abc").unwrap_err();

        assert_eq!(error, "\"abc\" is not a price, for example 9.99 or 9,99");
        assert!(parse_price("9.999").is_err());
        assert!(parse_price("").is_err());
    }
}
