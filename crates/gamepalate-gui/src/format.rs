use gamepalate_app::Timestamp;
use jiff::tz::TimeZone;

pub fn when(at: Timestamp, now: Timestamp) -> String {
    when_in(at, now, &TimeZone::system())
}

fn when_in(at: Timestamp, now: Timestamp, zone: &TimeZone) -> String {
    let (Ok(at), Ok(now)) = (
        jiff::Timestamp::from_millisecond(at.0),
        jiff::Timestamp::from_millisecond(now.0),
    ) else {
        return "unknown".to_string();
    };
    let at = at.to_zoned(zone.clone());
    let now = now.to_zoned(zone.clone());

    if at.date() == now.date() {
        at.strftime("today %H:%M").to_string()
    } else if at.year() == now.year() {
        at.strftime("%-d %b").to_string()
    } else {
        at.strftime("%-d %b %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);
    const HOUR: i64 = 3_600_000;

    fn when_utc(at: Timestamp) -> String {
        when_in(at, NOW, &TimeZone::UTC)
    }

    #[test]
    fn shows_the_time_for_today() {
        assert_eq!(when_utc(Timestamp(NOW.0 - 3 * HOUR)), "today 14:00");
    }

    #[test]
    fn shows_day_and_month_for_an_earlier_day_this_year() {
        assert_eq!(when_utc(Timestamp(NOW.0 - 48 * HOUR)), "31 Oct");
    }

    #[test]
    fn adds_the_year_for_an_earlier_year() {
        assert_eq!(when_utc(Timestamp(NOW.0 - 365 * 24 * HOUR)), "2 Nov 2025");
    }
}
