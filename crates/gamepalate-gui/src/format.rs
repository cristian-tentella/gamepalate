use gamepalate_app::{SearchOptions, SortBy, Timestamp, VoteKind};
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

pub fn filters(options: &SearchOptions) -> String {
    let filters = &options.filters;
    let mut parts = vec![match &options.family {
        Some(family) => format!("From: {family}"),
        None => "From: All tastes".to_string(),
    }];

    if options.only_bets {
        parts.push("Only bets".to_string());
    }
    if filters.only_discounted {
        parts.push("On sale".to_string());
    }
    if let Some(percent) = filters.min_discount_percent {
        parts.push(format!("≥ {percent}% off"));
    }
    match (filters.min_price_cents, filters.max_price_cents) {
        (Some(min), Some(max)) => parts.push(format!("{}–{}", euros(min), euros(max))),
        (Some(min), None) => parts.push(format!("≥ {}", euros(min))),
        (None, Some(max)) => parts.push(format!("≤ {}", euros(max))),
        (None, None) => {}
    }
    match (filters.min_reviews, filters.max_reviews) {
        (Some(min), Some(max)) => parts.push(format!("{}–{} reviews", count(min), count(max))),
        (Some(min), None) => parts.push(format!("≥ {} reviews", count(min))),
        (None, Some(max)) => parts.push(format!("≤ {} reviews", count(max))),
        (None, None) => {}
    }
    if let Some(percent) = filters.min_positive_percent {
        parts.push(format!("≥ {percent}% positive"));
    }
    match filters.without_early_access {
        Some(true) => parts.push("No early access".to_string()),
        Some(false) => parts.push("Early access included".to_string()),
        None => {}
    }
    if filters.without_voted {
        parts.push("Not voted".to_string());
    }
    if !filters.tags.is_empty() {
        parts.push(format!("Tags: {}", filters.tags.join(", ")));
    }
    if !filters.without_tags.is_empty() {
        parts.push(format!("Without: {}", filters.without_tags.join(", ")));
    }
    match options.sort {
        SortBy::Relevance => {}
        SortBy::Discount => parts.push("By discount".to_string()),
        SortBy::Price => parts.push("By price".to_string()),
    }
    if let Some(limit) = options.limit {
        parts.push(format!("Top {limit}"));
    }

    parts.join(" · ")
}

pub fn votes_this_session(votes: usize) -> String {
    match votes {
        1 => "1 vote this session".to_string(),
        votes => format!("{votes} votes this session"),
    }
}

pub fn euros(cents: u32) -> String {
    match cents % 100 {
        0 => format!("€{}", count(cents / 100)),
        rest => format!("€{}.{rest:02}", count(cents / 100)),
    }
}

pub fn count(number: u32) -> String {
    let digits = number.to_string();
    let mut grouped = String::new();
    for (position, digit) in digits.chars().enumerate() {
        if position > 0 && (digits.len() - position).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

pub fn vote(kind: VoteKind) -> &'static str {
    match kind {
        VoteKind::WouldTry => "Would try",
        VoteKind::Maybe => "Maybe",
        VoteKind::NotForMe => "Not for me",
        VoteKind::Known => "Already know it",
    }
}

pub fn voted(name: &str, kind: VoteKind) -> String {
    format!("{name}: voted {}. Not saved yet.", vote(kind))
}

pub fn eneba_url(name: &str) -> String {
    format!(
        "https://www.eneba.com/it/store/all?drms%5B%5D=steam&regions%5B%5D=global&regions%5B%5D=europe&sortBy=PRICE_ASC&text={}",
        search_text(name)
    )
}

pub fn hltb_url(name: &str) -> String {
    format!("https://howlongtobeat.com/?q={}", search_text(name))
}

fn search_text(name: &str) -> String {
    let name: String = name.chars().filter(|c| !"™®©".contains(*c)).collect();
    let mut encoded = String::new();
    for byte in name.trim().bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
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

    #[test]
    fn says_all_tastes_without_filters() {
        assert_eq!(filters(&SearchOptions::default()), "From: All tastes");
    }

    #[test]
    fn lists_the_active_filters_in_order() {
        let mut options = SearchOptions::default();
        options.filters.only_discounted = true;
        options.filters.max_price_cents = Some(1500);

        assert_eq!(filters(&options), "From: All tastes · On sale · ≤ €15");
    }

    #[test]
    fn names_the_family_ranges_and_tags() {
        let mut options = SearchOptions {
            family: Some("azione".to_string()),
            sort: SortBy::Price,
            ..SearchOptions::default()
        };
        options.filters.min_price_cents = Some(499);
        options.filters.max_price_cents = Some(2000);
        options.filters.min_reviews = Some(1000);
        options.filters.tags = vec!["Metroidvania".to_string(), "Souls-like".to_string()];

        assert_eq!(
            filters(&options),
            "From: azione · €4.99–€20 · ≥ 1,000 reviews · Tags: Metroidvania, Souls-like · By price"
        );
    }

    #[test]
    fn counts_votes_in_singular_and_plural() {
        assert_eq!(votes_this_session(1), "1 vote this session");
        assert_eq!(votes_this_session(12), "12 votes this session");
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(count(999), "999");
        assert_eq!(count(43_506), "43,506");
        assert_eq!(count(1_234_567), "1,234,567");
    }

    #[test]
    fn says_the_vote_is_not_saved_yet() {
        assert_eq!(
            voted("INFRA", VoteKind::WouldTry),
            "INFRA: voted Would try. Not saved yet."
        );
    }

    #[test]
    fn searches_by_name_without_trademarks() {
        assert_eq!(
            hltb_url("Hades™ II: Café & Co. "),
            "https://howlongtobeat.com/?q=Hades%20II%3A%20Caf%C3%A9%20%26%20Co."
        );
        assert!(eneba_url("Celeste").ends_with("&text=Celeste"));
    }
}
