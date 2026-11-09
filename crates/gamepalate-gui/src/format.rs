use gamepalate_app::{
    Blame, Controller, Currency, Price, Reviews, SearchOptions, SteamDeck, Timestamp, VoteKind,
};
use jiff::{civil::Date, tz::TimeZone};

const FEW_REVIEWS_GAP: f64 = 5.0;

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

// Whole filters while they fit, then …; None when the summary fits as it is.
pub fn short_filters(summary: &str, most: usize) -> Option<String> {
    if summary.chars().count() <= most {
        return None;
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut length = 0;
    for part in summary.split(" · ") {
        let added = part.chars().count() + if kept.is_empty() { 0 } else { 3 };
        if length + added > most {
            break;
        }
        length += added;
        kept.push(part);
    }

    Some(if kept.is_empty() {
        let cut: String = summary.chars().take(most).collect();
        format!("{}…", cut.trim_end())
    } else {
        format!("{} · …", kept.join(" · "))
    })
}

pub fn filters(options: &SearchOptions, currency: &Currency) -> Option<String> {
    let filters = &options.filters;
    let mut parts = Vec::new();

    if options.only_bets {
        parts.push("Bets only".to_string());
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
    if let Some(limit) = options.limit {
        parts.push(format!("Top {limit}"));
    }

    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn empty_because(blame: Option<&Blame>) -> String {
    let Some(blame) = blame else {
        return "No game passes your filters".to_string();
    };
    let games = match blame.discarded {
        1 => "1 game".to_string(),
        discarded => format!("{} games", count(discarded as u32)),
    };

    format!(
        "No game passes your filters. Try loosening {}, which removed {games}.",
        blame.filter
    )
}

pub fn votes_this_session(session: usize, last_day: usize) -> String {
    let session = match session {
        1 => "1 vote this session".to_string(),
        votes => format!("{votes} votes this session"),
    };
    format!("{session} · {last_day} in the last 24 hours")
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

pub fn short_count(number: u32) -> String {
    if number < 1_000 {
        return number.to_string();
    }
    let (value, unit) = if number < 999_500 {
        (f64::from(number) / 1e3, "k")
    } else {
        (f64::from(number) / 1e6, "M")
    };

    if value < 9.95 {
        let short = format!("{value:.1}");
        format!("{}{unit}", short.trim_end_matches(".0"))
    } else {
        format!("{}{unit}", value.round())
    }
}

pub fn currency_symbol(currency: &Currency) -> String {
    format!("{}{}", currency.prefix.trim(), currency.suffix.trim())
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

pub fn day_time(at: Timestamp) -> String {
    day_time_in(at, &TimeZone::system())
}

fn day_time_in(at: Timestamp, zone: &TimeZone) -> String {
    match jiff::Timestamp::from_millisecond(at.0) {
        Ok(at) => at
            .to_zoned(zone.clone())
            .strftime("%-d %b %H:%M")
            .to_string(),
        Err(_) => "unknown".to_string(),
    }
}

pub fn time_left(left: std::time::Duration) -> String {
    match (left.as_secs() + 30) / 60 {
        0 => "Less than a minute left".to_string(),
        1 => "About 1 minute left".to_string(),
        minutes => format!("About {minutes} minutes left"),
    }
}

pub fn original_price(price: &Price, currency: &Currency) -> String {
    currency.format(price.original_cents)
}

pub fn ends(end: Timestamp) -> Option<String> {
    ends_in(end, &TimeZone::system(), jiff::Zoned::now().date())
}

fn ends_in(end: Timestamp, zone: &TimeZone, today: Date) -> Option<String> {
    let end = jiff::Timestamp::from_millisecond(end.0)
        .ok()?
        .to_zoned(zone.clone());
    let days = today.until(end.date()).ok()?.get_days();
    Some(match days {
        ..=0 => "Ends today".to_string(),
        1 => "Ends tomorrow".to_string(),
        2..=7 => format!("Ends in {days} days"),
        _ => end.strftime("Ends %-d %b").to_string(),
    })
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
        format!("positive of {} {noun}", short_count(reviews.total)),
    )
}

pub fn affinity(affinity: f64) -> String {
    let times = format!("{affinity:.1}");
    format!("{}×", times.trim_end_matches(".0"))
}

pub fn few_reviews(reviews: &Reviews, quality: f64) -> Option<String> {
    if reviews.total == 0 {
        return None;
    }
    let percent = (f64::from(reviews.positive) * 100.0 / f64::from(reviews.total)).round();
    let counted = (quality * 100.0).round();
    let (noun, verb) = if reviews.total == 1 {
        ("review", "it counts")
    } else {
        ("reviews", "they count")
    };

    (percent - counted >= FEW_REVIEWS_GAP).then(|| {
        format!(
            "Only {} {noun}, so {verb} as {counted}%",
            count(reviews.total)
        )
    })
}

pub fn bet(every: usize) -> String {
    format!("A game your votes would skip. 1 in {every} is a Bet.")
}

pub fn screenshot_position(index: usize, count: usize) -> String {
    format!("{} of {count}", index + 1)
}

pub fn steam_deck(deck: SteamDeck) -> &'static str {
    match deck {
        SteamDeck::Verified => "Deck Verified",
        SteamDeck::Playable => "Deck Playable",
        SteamDeck::Unsupported => "Deck Unsupported",
    }
}

pub fn features(
    deck: Option<SteamDeck>,
    modes: &[String],
    controller: Option<Controller>,
) -> Vec<&str> {
    let controller = controller.map(|controller| match controller {
        Controller::Full => "Full controller",
        Controller::Partial => "Partial controller",
    });

    deck.map(steam_deck)
        .into_iter()
        .chain(modes.iter().map(String::as_str))
        .chain(controller)
        .collect()
}

pub fn developers(developers: &[String]) -> Option<String> {
    (!developers.is_empty()).then(|| developers.join(", "))
}

pub fn byline(year: Option<String>, developers: Option<String>) -> Option<String> {
    match (year, developers) {
        (Some(year), Some(developers)) => Some(format!("{year} · {developers}")),
        (year, developers) => year.or(developers),
    }
}

pub fn year(release: Option<Timestamp>) -> Option<String> {
    let release = jiff::Timestamp::from_millisecond(release?.0).ok()?;
    Some(release.to_zoned(TimeZone::UTC).year().to_string())
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
    format!("{name}: voted {}", vote(kind))
}

pub fn voted_day(date: &str, today: Date) -> String {
    let Ok(day) = date.parse::<Date>() else {
        return date.to_string();
    };

    if day == today {
        "today".to_string()
    } else if day.year() == today.year() {
        day.strftime("%-d %b").to_string()
    } else {
        day.strftime("%-d %b %Y").to_string()
    }
}

pub fn families_made(count: usize) -> String {
    let families = if count == 1 { "family" } else { "families" };
    format!(
        "Made {count} {families} from the games you play. You can rename, change or remove them in Profile"
    )
}

pub fn automatic_note(date: &str, today: Date) -> String {
    let made = match date.parse::<Date>() {
        Ok(_) => match voted_day(date, today).as_str() {
            "today" => "Made today".to_string(),
            day => format!("Made on {day}"),
        },
        Err(_) => "Made".to_string(),
    };

    format!("{made} from your games. Edit it to keep it")
}

pub fn bets_summary(would_try: usize, bets: usize, others: Option<f64>) -> String {
    let share = would_try as f64 / bets as f64;
    let mut summary = format!("Bets: {would_try} of {bets} Would try ({})", percent(share));
    if let Some(others) = others {
        summary.push_str(&format!(
            ", other votes since the first Bet: {}",
            percent(others)
        ));
    }
    summary
}

fn percent(share: f64) -> String {
    format!("{:.0}%", share * 100.0)
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
    use gamepalate_app::{Discount, SortBy};

    use super::*;

    #[test]
    fn a_deadline_has_day_month_and_time() {
        assert_eq!(
            day_time_in(Timestamp(1_793_369_460_000), &TimeZone::UTC),
            "30 Oct 14:11"
        );
    }

    #[test]
    fn the_time_left_is_rounded_to_the_minute() {
        use std::time::Duration;

        assert_eq!(
            time_left(Duration::from_secs(20)),
            "Less than a minute left"
        );
        assert_eq!(time_left(Duration::from_secs(80)), "About 1 minute left");
        assert_eq!(
            time_left(Duration::from_secs(12 * 60 + 10)),
            "About 12 minutes left"
        );
    }

    const NOW: Timestamp = Timestamp(1_793_638_800_000);
    const HOUR: i64 = 3_600_000;

    #[test]
    fn the_vote_day_drops_the_year_of_today() {
        let today = Date::constant(2026, 10, 30);

        assert_eq!(voted_day("2026-10-30", today), "today");
        assert_eq!(voted_day("2026-10-12", today), "12 Oct");
        assert_eq!(voted_day("2025-03-01", today), "1 Mar 2025");
        assert_eq!(voted_day("someday", today), "someday");
    }

    #[test]
    fn a_long_filter_summary_keeps_the_whole_filters_that_fit() {
        let summary = "≥ 1,350 reviews · ≥ 78% positive · No early access · Without: Anime, Dating Sim, Noir, Visual Novel";

        assert_eq!(
            short_filters(summary, 60).as_deref(),
            Some("≥ 1,350 reviews · ≥ 78% positive · No early access · …")
        );
        assert_eq!(short_filters("On sale · Bets only", 60), None);
        assert_eq!(
            short_filters("Without: Anime, Dating Sim, Noir", 10).as_deref(),
            Some("Without: A…")
        );
    }

    #[test]
    fn the_families_notice_counts_them() {
        assert_eq!(
            families_made(4),
            "Made 4 families from the games you play. You can rename, change or remove them in Profile"
        );
        assert!(families_made(1).starts_with("Made 1 family from"));
    }

    #[test]
    fn the_automatic_note_says_the_day() {
        let today = Date::constant(2026, 11, 12);

        assert_eq!(
            automatic_note("2026-11-10", today),
            "Made on 10 Nov from your games. Edit it to keep it"
        );
        assert_eq!(
            automatic_note("2026-11-12", today),
            "Made today from your games. Edit it to keep it"
        );
        assert_eq!(
            automatic_note("2025-03-01", today),
            "Made on 1 Mar 2025 from your games. Edit it to keep it"
        );
        assert_eq!(
            automatic_note("someday", today),
            "Made from your games. Edit it to keep it"
        );
    }

    #[test]
    fn the_bets_summary_compares_with_the_other_votes() {
        assert_eq!(
            bets_summary(9, 51, Some(0.31)),
            "Bets: 9 of 51 Would try (18%), other votes since the first Bet: 31%"
        );
        assert_eq!(bets_summary(1, 2, None), "Bets: 1 of 2 Would try (50%)");
    }

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
    fn says_nothing_without_filters_and_leaves_out_the_source_and_the_order() {
        let mut options = SearchOptions {
            signals_only: true,
            sort: SortBy::Price,
            ..SearchOptions::default()
        };
        options.filters.without_voted = true;

        assert_eq!(filters(&options, &Currency::italian()), None);
    }

    #[test]
    fn names_the_strictest_filter_when_nothing_passes() {
        let blame = |discarded| Blame {
            filter: "max price",
            discarded,
        };

        assert_eq!(
            empty_because(Some(&blame(1234))),
            "No game passes your filters. Try loosening max price, which removed 1,234 games."
        );
        assert_eq!(
            empty_because(Some(&blame(1))),
            "No game passes your filters. Try loosening max price, which removed 1 game."
        );
        assert_eq!(empty_because(None), "No game passes your filters");
    }

    #[test]
    fn lists_the_active_filters_in_order() {
        let mut options = SearchOptions::default();
        options.filters.only_discounted = true;
        options.filters.max_price_cents = Some(1500);

        assert_eq!(
            filters(&options, &Currency::italian()),
            Some("On sale · ≤ 15,00€".to_string())
        );
        assert_eq!(
            filters(&options, &dollars()),
            Some("On sale · ≤ $15.00".to_string())
        );
    }

    #[test]
    fn names_the_ranges_and_tags() {
        let mut options = SearchOptions::default();
        options.filters.min_price_cents = Some(499);
        options.filters.max_price_cents = Some(2000);
        options.filters.min_reviews = Some(1000);
        options.filters.tags = vec!["Metroidvania".to_string(), "Souls-like".to_string()];

        assert_eq!(
            filters(&options, &Currency::italian()),
            Some("4,99€–20,00€ · ≥ 1,000 reviews · Tags: Metroidvania, Souls-like".to_string())
        );
    }

    #[test]
    fn the_currency_symbol_drops_the_spaces() {
        assert_eq!(currency_symbol(&Currency::italian()), "€");
        assert_eq!(currency_symbol(&dollars()), "$");
    }

    #[test]
    fn counts_votes_in_singular_and_plural() {
        assert_eq!(
            votes_this_session(1, 5),
            "1 vote this session · 5 in the last 24 hours"
        );
        assert_eq!(
            votes_this_session(12, 40),
            "12 votes this session · 40 in the last 24 hours"
        );
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
    fn says_when_the_discount_ends() {
        let today = Date::constant(2026, 11, 2);
        let ends = |days: i64| ends_in(Timestamp(NOW.0 + days * 24 * HOUR), &TimeZone::UTC, today);

        assert_eq!(ends(0), Some("Ends today".to_string()));
        assert_eq!(ends(1), Some("Ends tomorrow".to_string()));
        assert_eq!(ends(2), Some("Ends in 2 days".to_string()));
        assert_eq!(ends(7), Some("Ends in 7 days".to_string()));
        assert_eq!(ends(8), Some("Ends 10 Nov".to_string()));
        assert_eq!(ends(-1), Some("Ends today".to_string()));
    }

    #[test]
    fn the_original_price_ignores_the_discount() {
        let price = Price {
            original_cents: 1999,
            discount: Some(Discount {
                percent: 85,
                final_cents: 299,
                end: None,
            }),
        };

        let currency = Currency::italian();

        assert_eq!(original_price(&price, &currency), currency.format(1999));
    }

    #[test]
    fn shortens_counts_over_a_thousand() {
        assert_eq!(short_count(999), "999");
        assert_eq!(short_count(1_000), "1k");
        assert_eq!(short_count(1_540), "1.5k");
        assert_eq!(short_count(9_949), "9.9k");
        assert_eq!(short_count(9_950), "10k");
        assert_eq!(short_count(11_039), "11k");
        assert_eq!(short_count(999_499), "999k");
        assert_eq!(short_count(999_500), "1M");
        assert_eq!(short_count(2_345_678), "2.3M");
    }

    #[test]
    fn rounds_the_positive_reviews() {
        let reviews = Reviews {
            total: 43_506,
            positive: 40_461,
        };

        assert_eq!(
            super::reviews(&reviews),
            ("93%".to_string(), "positive of 44k reviews".to_string())
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
    fn writes_the_affinity_as_times() {
        assert_eq!(affinity(6.5), "6.5×");
        assert_eq!(affinity(4.0), "4×");
    }

    #[test]
    fn warns_only_when_few_reviews_count_less() {
        let few = Reviews {
            total: 20,
            positive: 19,
        };
        let many = Reviews {
            total: 43_506,
            positive: 40_461,
        };

        assert_eq!(
            few_reviews(&few, 0.764).unwrap(),
            "Only 20 reviews, so they count as 76%"
        );
        assert_eq!(few_reviews(&many, 0.927), None);
        assert_eq!(
            few_reviews(
                &Reviews {
                    total: 0,
                    positive: 0
                },
                0.0
            ),
            None
        );
    }

    #[test]
    fn says_how_often_a_bet_comes() {
        assert_eq!(bet(5), "A game your votes would skip. 1 in 5 is a Bet.");
    }

    #[test]
    fn counts_the_screenshots_from_one() {
        assert_eq!(screenshot_position(2, 10), "3 of 10");
    }

    #[test]
    fn names_the_steam_deck_support() {
        assert_eq!(steam_deck(SteamDeck::Verified), "Deck Verified");
        assert_eq!(steam_deck(SteamDeck::Unsupported), "Deck Unsupported");
    }

    #[test]
    fn lists_deck_modes_and_controller_in_order() {
        let modes = ["Single-player".to_string(), "Online Co-op".to_string()];

        assert_eq!(
            features(Some(SteamDeck::Verified), &modes, Some(Controller::Full)),
            [
                "Deck Verified",
                "Single-player",
                "Online Co-op",
                "Full controller"
            ]
        );
        assert_eq!(
            features(None, &[], Some(Controller::Partial)),
            ["Partial controller"]
        );
        assert!(features(None, &[], None).is_empty());
    }

    #[test]
    fn names_the_developers() {
        let developers = ["Live Wire".to_string(), "Adglobe".to_string()];

        assert_eq!(
            super::developers(&developers).unwrap(),
            "Live Wire, Adglobe"
        );
        assert_eq!(super::developers(&[]), None);
    }

    #[test]
    fn the_byline_joins_year_and_developers() {
        let developers = || Some("Cyanide Studio".to_string());

        assert_eq!(
            byline(Some("2018".to_string()), developers()).unwrap(),
            "2018 · Cyanide Studio"
        );
        assert_eq!(byline(None, developers()).unwrap(), "Cyanide Studio");
        assert_eq!(byline(Some("2018".to_string()), None).unwrap(), "2018");
        assert_eq!(byline(None, None), None);
    }

    #[test]
    fn takes_the_release_year() {
        assert_eq!(year(Some(Timestamp(1_624_233_600_000))).unwrap(), "2021");
        assert_eq!(year(None), None);
    }

    #[test]
    fn says_what_was_voted() {
        assert_eq!(voted("INFRA", VoteKind::WouldTry), "INFRA: voted Would try");
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
