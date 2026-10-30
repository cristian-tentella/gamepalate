use gamepalate_app::{
    Blame, Controller, Currency, Italian, Price, Reviews, SearchOptions, SortBy, SteamDeck,
    Timestamp, VoteKind,
};
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

pub fn filters(options: &SearchOptions, currency: &Currency) -> String {
    let filters = &options.filters;
    let mut parts = vec![match (&options.family, options.signals_only) {
        (Some(family), _) => format!("From: {family}"),
        (None, true) => "From: Signals only".to_string(),
        (None, false) => "From: All tastes".to_string(),
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
        (Some(min), Some(max)) => {
            parts.push(format!("{}–{}", currency.format(min), currency.format(max)))
        }
        (Some(min), None) => parts.push(format!("≥ {}", currency.format(min))),
        (None, Some(max)) => parts.push(format!("≤ {}", currency.format(max))),
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

pub fn empty_because(blame: Option<&Blame>) -> String {
    let Some(blame) = blame else {
        return "No game passes your filters.".to_string();
    };
    let games = match blame.discarded {
        1 => "1 game".to_string(),
        discarded => format!("{} games", count(discarded as u32)),
    };

    format!(
        "No game passes your filters. The strictest is {}: it removed {games}.",
        blame.filter
    )
}

pub fn votes_this_session(votes: usize) -> String {
    match votes {
        1 => "1 vote this session".to_string(),
        votes => format!("{votes} votes this session"),
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

pub fn price(price: &Price, currency: &Currency) -> String {
    match price.discount {
        Some(discount) => currency.format(discount.final_cents),
        None if price.original_cents == 0 => "Free".to_string(),
        None => currency.format(price.original_cents),
    }
}

pub fn discount(percent: u8) -> String {
    format!("−{percent}%")
}

pub fn until(end: Timestamp) -> String {
    until_in(end, &TimeZone::system())
}

fn until_in(end: Timestamp, zone: &TimeZone) -> String {
    match jiff::Timestamp::from_millisecond(end.0) {
        Ok(end) => end
            .to_zoned(zone.clone())
            .strftime("until %-d %b")
            .to_string(),
        Err(_) => String::new(),
    }
}

pub fn reviews(reviews: &Reviews) -> (String, String) {
    if reviews.total == 0 {
        return (String::new(), "No reviews yet".to_string());
    }
    let percent = (f64::from(reviews.positive) * 100.0 / f64::from(reviews.total)).round();
    let noun = if reviews.total == 1 {
        "review"
    } else {
        "reviews"
    };

    (
        format!("{percent}%"),
        format!(" positive of {} {noun}", count(reviews.total)),
    )
}

pub fn affinity(affinity: f64) -> String {
    format!("{affinity:.1}×")
}

pub fn affinity_share(affinity: f64) -> f32 {
    (affinity.max(1.0).log10() as f32).clamp(0.0, 1.0)
}

pub fn quality(quality: f64) -> &'static str {
    match quality {
        q if q >= 0.9 => "excellent",
        q if q >= 0.8 => "very good",
        q if q >= 0.7 => "good",
        q if q >= 0.5 => "mixed",
        _ => "poor",
    }
}

pub fn bet(every: usize) -> String {
    format!("It's a Bet: 1 game in {every} is one your votes would skip.")
}

pub fn screenshot_position(index: usize, count: usize) -> String {
    format!("{} of {count}", index + 1)
}

pub fn steam_deck(deck: SteamDeck) -> &'static str {
    match deck {
        SteamDeck::Verified => "Steam Deck: Verified",
        SteamDeck::Playable => "Steam Deck: Playable",
        SteamDeck::Unsupported => "Steam Deck: Unsupported",
    }
}

pub fn italian(italian: Option<Italian>) -> String {
    let Some(italian) = italian else {
        return "No Italian".to_string();
    };
    let mut parts = vec!["interface"];
    if italian.subtitles {
        parts.push("subtitles");
    }
    if italian.audio {
        parts.push("audio");
    }
    format!("Italian: {}", parts.join(", "))
}

pub fn play(modes: &[String], controller: Option<Controller>) -> Option<String> {
    let controller = controller.map(|controller| match controller {
        Controller::Full => "Full controller support",
        Controller::Partial => "Partial controller support",
    });
    let parts: Vec<&str> = modes.iter().map(String::as_str).chain(controller).collect();

    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn made_by(developers: &[String], release: Option<Timestamp>) -> Option<String> {
    let year = release
        .and_then(|release| jiff::Timestamp::from_millisecond(release.0).ok())
        .map(|release| release.to_zoned(TimeZone::UTC).year().to_string());
    let parts: Vec<String> = (!developers.is_empty())
        .then(|| developers.join(", "))
        .into_iter()
        .chain(year)
        .collect();

    (!parts.is_empty()).then(|| parts.join(" · "))
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
    format!("{name}: voted {}.", vote(kind))
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
    use gamepalate_app::Discount;

    use super::*;

    const NOW: Timestamp = Timestamp(1_793_638_800_000);
    const HOUR: i64 = 3_600_000;

    fn when_utc(at: Timestamp) -> String {
        when_in(at, NOW, &TimeZone::UTC)
    }

    fn dollars() -> Currency {
        Currency {
            prefix: "$".to_string(),
            suffix: String::new(),
            decimal_separator: Some('.'),
            group_separator: Some(','),
        }
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
        assert_eq!(
            filters(&SearchOptions::default(), &Currency::italian()),
            "From: All tastes"
        );
    }

    #[test]
    fn says_signals_only_and_leaves_the_voted_games_out() {
        let mut options = SearchOptions {
            signals_only: true,
            ..SearchOptions::default()
        };
        options.filters.without_voted = true;

        assert_eq!(
            filters(&options, &Currency::italian()),
            "From: Signals only"
        );
    }

    #[test]
    fn names_the_strictest_filter_when_nothing_passes() {
        let blame = |discarded| Blame {
            filter: "max price",
            discarded,
        };

        assert_eq!(
            empty_because(Some(&blame(1234))),
            "No game passes your filters. The strictest is max price: it removed 1,234 games."
        );
        assert_eq!(
            empty_because(Some(&blame(1))),
            "No game passes your filters. The strictest is max price: it removed 1 game."
        );
        assert_eq!(empty_because(None), "No game passes your filters.");
    }

    #[test]
    fn lists_the_active_filters_in_order() {
        let mut options = SearchOptions::default();
        options.filters.only_discounted = true;
        options.filters.max_price_cents = Some(1500);

        assert_eq!(
            filters(&options, &Currency::italian()),
            "From: All tastes · On sale · ≤ 15,00€"
        );
        assert_eq!(
            filters(&options, &dollars()),
            "From: All tastes · On sale · ≤ $15.00"
        );
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
            filters(&options, &Currency::italian()),
            "From: azione · 4,99€–20,00€ · ≥ 1,000 reviews · Tags: Metroidvania, Souls-like · By price"
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
    fn shows_the_discounted_price() {
        let price = Price {
            original_cents: 2499,
            discount: Some(Discount {
                percent: 40,
                final_cents: 1499,
                end: None,
            }),
        };

        assert_eq!(super::price(&price, &Currency::italian()), "14,99€");
        assert_eq!(super::price(&price, &dollars()), "$14.99");
        assert_eq!(discount(40), "−40%");
    }

    #[test]
    fn a_currency_without_decimals_shows_none() {
        let yen = Currency {
            prefix: "¥".to_string(),
            suffix: String::new(),
            decimal_separator: None,
            group_separator: Some(','),
        };
        let price = Price {
            original_cents: 902_000,
            discount: None,
        };

        assert_eq!(super::price(&price, &yen), "¥9,020");
    }

    #[test]
    fn says_free_without_a_price() {
        let price = Price {
            original_cents: 0,
            discount: None,
        };

        assert_eq!(super::price(&price, &Currency::italian()), "Free");
    }

    #[test]
    fn shows_when_the_discount_ends() {
        assert_eq!(until_in(NOW, &TimeZone::UTC), "until 2 Nov");
    }

    #[test]
    fn rounds_the_positive_reviews() {
        let reviews = Reviews {
            total: 43_506,
            positive: 40_461,
        };

        assert_eq!(
            super::reviews(&reviews),
            ("93%".to_string(), " positive of 43,506 reviews".to_string())
        );
    }

    #[test]
    fn says_when_there_are_no_reviews() {
        let reviews = Reviews {
            total: 0,
            positive: 0,
        };

        assert_eq!(super::reviews(&reviews).1, "No reviews yet");
    }

    #[test]
    fn fills_the_affinity_bar_on_a_log_scale() {
        assert_eq!(affinity(6.5), "6.5×");
        assert_eq!(affinity_share(0.5), 0.0);
        assert!((affinity_share(6.5) - 0.81).abs() < 0.01);
        assert_eq!(affinity_share(40.0), 1.0);
    }

    #[test]
    fn names_the_quality() {
        assert_eq!(quality(0.95), "excellent");
        assert_eq!(quality(0.85), "very good");
        assert_eq!(quality(0.75), "good");
        assert_eq!(quality(0.6), "mixed");
        assert_eq!(quality(0.3), "poor");
    }

    #[test]
    fn says_how_often_a_bet_comes() {
        assert_eq!(
            bet(5),
            "It's a Bet: 1 game in 5 is one your votes would skip."
        );
    }

    #[test]
    fn counts_the_screenshots_from_one() {
        assert_eq!(screenshot_position(2, 10), "3 of 10");
    }

    #[test]
    fn names_the_steam_deck_support() {
        assert_eq!(steam_deck(SteamDeck::Verified), "Steam Deck: Verified");
        assert_eq!(
            steam_deck(SteamDeck::Unsupported),
            "Steam Deck: Unsupported"
        );
    }

    #[test]
    fn lists_what_is_in_italian() {
        assert_eq!(italian(None), "No Italian");
        assert_eq!(
            italian(Some(Italian {
                subtitles: false,
                audio: false
            })),
            "Italian: interface"
        );
        assert_eq!(
            italian(Some(Italian {
                subtitles: true,
                audio: true
            })),
            "Italian: interface, subtitles, audio"
        );
    }

    #[test]
    fn joins_the_modes_and_the_controller() {
        let modes = ["Single-player".to_string(), "Online Co-op".to_string()];

        assert_eq!(
            play(&modes, Some(Controller::Full)).unwrap(),
            "Single-player · Online Co-op · Full controller support"
        );
        assert_eq!(
            play(&[], Some(Controller::Partial)).unwrap(),
            "Partial controller support"
        );
        assert_eq!(play(&[], None), None);
    }

    #[test]
    fn names_the_developers_and_the_year() {
        let developers = ["Live Wire".to_string(), "Adglobe".to_string()];

        assert_eq!(
            made_by(&developers, Some(Timestamp(1_624_233_600_000))).unwrap(),
            "Live Wire, Adglobe · 2021"
        );
        assert_eq!(made_by(&developers, None).unwrap(), "Live Wire, Adglobe");
        assert_eq!(
            made_by(&[], Some(Timestamp(1_624_233_600_000))).unwrap(),
            "2021"
        );
        assert_eq!(made_by(&[], None), None);
    }

    #[test]
    fn says_what_was_voted() {
        assert_eq!(
            voted("INFRA", VoteKind::WouldTry),
            "INFRA: voted Would try."
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
