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
