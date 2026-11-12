use std::fmt;

use gamepalate_app::{FilterOptions, SearchOptions, SortBy, parse_price};
use iced::{
    Center, Element, Fill, Theme, padding,
    widget::{
        button, checkbox, column, combo_box, container, row, space, text, text_input, tooltip,
    },
};

use crate::{font, format, style};

const LABEL_WIDTH: f32 = 64.0;
const NUMBER_WIDTH: f32 = 72.0;
const RANGE_WIDTH: f32 = 196.0;
const TAG_WIDTH: f32 = 196.0;
const SPACING: f32 = 12.0;
const PANEL_WIDTH: f32 = 520.0;
const INFO: char = '\u{E88E}';
// A longer family name ends in … so the From menu keeps its width.
const FAMILY_NAME_CHARS: usize = 26;

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    AllTastes,
    Signals,
    Family(String),
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::AllTastes => formatter.write_str("All tastes"),
            Source::Signals => formatter.write_str("Signals only"),
            Source::Family(name) => formatter.write_str(&short_name(name)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sort(pub SortBy);

impl fmt::Display for Sort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.0 {
            SortBy::Relevance => "By relevance",
            SortBy::Discount => "By discount",
            SortBy::Price => "By price",
        })
    }
}

pub const SORTS: [Sort; 3] = [
    Sort(SortBy::Relevance),
    Sort(SortBy::Discount),
    Sort(SortBy::Price),
];

pub fn sources(families: Vec<String>) -> Vec<Source> {
    [Source::AllTastes, Source::Signals]
        .into_iter()
        .chain(families.into_iter().map(Source::Family))
        .collect()
}

fn short_name(name: &str) -> String {
    if name.chars().count() <= FAMILY_NAME_CHARS {
        return name.to_string();
    }
    let kept: String = name.chars().take(FAMILY_NAME_CHARS - 1).collect();
    format!("{}…", kept.trim_end())
}

pub fn source(options: &SearchOptions) -> Source {
    match (&options.family, options.signals_only) {
        (Some(name), _) => Source::Family(name.clone()),
        (None, true) => Source::Signals,
        (None, false) => Source::AllTastes,
    }
}

pub fn with_source(options: &SearchOptions, source: Source) -> SearchOptions {
    let (family, signals_only) = match source {
        Source::AllTastes => (None, false),
        Source::Signals => (None, true),
        Source::Family(name) => (Some(name), false),
    };

    SearchOptions {
        family,
        signals_only,
        ..options.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    MaxPrice,
    MinPrice,
    MinDiscount,
    MinReviews,
    MaxReviews,
    MinPositive,
}

const FIELDS: [Field; 6] = [
    Field::MaxPrice,
    Field::MinPrice,
    Field::MinDiscount,
    Field::MinReviews,
    Field::MaxReviews,
    Field::MinPositive,
];

#[derive(Debug, Clone)]
pub enum Message {
    OnSale(bool),
    OnlyBets(bool),
    EarlyAccess(bool),
    Input(Field, String),
    AddTag(String),
    RemoveTag(String),
    AddWithoutTag(String),
    RemoveWithoutTag(String),
    Reset,
    Cancel,
    Done,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Stay,
    Close,
    Search(SearchOptions),
}

pub struct Panel {
    without_early_access: bool,
    placeholders: [String; 6],
    currency: String,
    filters: FilterOptions,
    only_bets: bool,
    texts: [String; 6],
    error: Option<(Field, String)>,
    tags: combo_box::State<String>,
    without_tags: combo_box::State<String>,
}

impl Panel {
    pub fn new(
        tags: Vec<String>,
        defaults: &FilterOptions,
        currency: String,
        options: &SearchOptions,
    ) -> Panel {
        let mut panel = Panel {
            without_early_access: defaults.without_early_access.unwrap_or(false),
            placeholders: placeholders(defaults),
            currency,
            filters: FilterOptions::default(),
            only_bets: false,
            texts: Default::default(),
            error: None,
            tags: combo_box::State::new(tags.clone()),
            without_tags: combo_box::State::new(tags),
        };
        panel.show(options);
        panel
    }

    pub fn show(&mut self, options: &SearchOptions) {
        self.filters = options.filters.clone();
        self.filters.only_discounted |= self.filters.min_discount_percent.is_some();
        self.only_bets = options.only_bets;
        self.texts = texts(&options.filters);
        self.error = None;
    }

    pub fn update(&mut self, options: &SearchOptions, message: Message) -> Action {
        let filters = &mut self.filters;
        match message {
            Message::OnSale(on) => {
                filters.only_discounted = on;
                if !on {
                    self.texts[index(Field::MinDiscount)].clear();
                    self.forget_error(Field::MinDiscount);
                }
            }
            Message::OnlyBets(on) => self.only_bets = on,
            Message::EarlyAccess(included) => {
                filters.without_early_access =
                    Some(!included).filter(|without| *without != self.without_early_access);
            }
            Message::Input(field, value) => {
                self.texts[index(field)] = value;
                self.forget_error(field);
            }
            Message::AddTag(tag) => add(&mut filters.tags, tag),
            Message::RemoveTag(tag) => filters.tags.retain(|kept| *kept != tag),
            Message::AddWithoutTag(tag) => add(&mut filters.without_tags, tag),
            Message::RemoveWithoutTag(tag) => filters.without_tags.retain(|kept| *kept != tag),
            Message::Reset => {
                *filters = FilterOptions {
                    without_voted: filters.without_voted,
                    ..FilterOptions::default()
                };
                self.only_bets = false;
                self.texts = texts(filters);
                self.error = None;
            }
            Message::Cancel => return Action::Close,
            Message::Done => match with_texts(filters, &self.texts) {
                Ok(filters) => {
                    let changed = SearchOptions {
                        filters,
                        only_bets: self.only_bets,
                        ..options.clone()
                    };
                    return if changed == *options {
                        Action::Close
                    } else {
                        Action::Search(changed)
                    };
                }
                Err(error) => self.error = Some(error),
            },
        }

        Action::Stay
    }

    pub fn view(&self) -> Element<'_, Message> {
        let filters = &self.filters;
        let on_sale = filters.only_discounted;

        let price = self.line(
            "Price",
            row![
                self.range(Field::MinPrice, Field::MaxPrice, Some(&self.currency)),
                checkbox(on_sale)
                    .label("On sale, at least")
                    .text_size(font::BAR_SIZE)
                    .on_toggle(Message::OnSale),
                self.number(Field::MinDiscount, on_sale),
                text("%").size(font::BAR_SIZE),
            ]
            .spacing(8)
            .align_y(Center),
            &[Field::MinPrice, Field::MaxPrice, Field::MinDiscount],
        );

        let reviews = self.line(
            "Reviews",
            row![
                self.range(Field::MinReviews, Field::MaxReviews, None),
                text("At least").size(font::BAR_SIZE),
                self.number(Field::MinPositive, true),
                text("% positive").size(font::BAR_SIZE),
            ]
            .spacing(8)
            .align_y(Center),
            &[Field::MinReviews, Field::MaxReviews, Field::MinPositive],
        );

        let tags = row![
            label("Tags"),
            tag_picker(
                "with tag",
                &self.tags,
                &filters.tags,
                Message::AddTag,
                Message::RemoveTag
            ),
            tag_picker(
                "without tag",
                &self.without_tags,
                &filters.without_tags,
                Message::AddWithoutTag,
                Message::RemoveWithoutTag,
            ),
        ]
        .spacing(SPACING);

        let included = !filters
            .without_early_access
            .unwrap_or(self.without_early_access);
        let bets = tooltip(
            text(INFO)
                .font(font::ICONS)
                .size(font::icon(font::NOTE_SIZE)),
            container(text("Games your votes would rank low").size(font::NOTE_SIZE))
                .padding([6, 10])
                .style(style::popup),
            tooltip::Position::Bottom,
        );
        let choices = row![
            label(""),
            checkbox(included)
                .label("Include early access")
                .text_size(font::BAR_SIZE)
                .on_toggle(Message::EarlyAccess),
            row![
                checkbox(self.only_bets)
                    .label("Bets only")
                    .text_size(font::BAR_SIZE)
                    .on_toggle(Message::OnlyBets),
                bets,
            ]
            .spacing(6)
            .align_y(Center),
        ]
        .spacing(SPACING)
        .align_y(Center);

        let actions = row![
            action("Reset", style::text_button, Message::Reset),
            space().width(Fill),
            action("Cancel", style::text_button, Message::Cancel),
            action("Done", style::primary, Message::Done),
        ]
        .spacing(8)
        .align_y(Center);

        column![price, reviews, tags, choices, actions]
            .spacing(14)
            .width(PANEL_WIDTH)
            .into()
    }

    fn line<'a>(
        &'a self,
        name: &'a str,
        content: impl Into<Element<'a, Message>>,
        fields: &[Field],
    ) -> Element<'a, Message> {
        let error = self
            .error
            .as_ref()
            .filter(|(wrong, _)| fields.contains(wrong))
            .map(|(_, error)| {
                container(text(error).size(font::NOTE_SIZE).style(text::danger))
                    .padding(padding::left(LABEL_WIDTH + SPACING))
            });

        column![
            row![label(name), content.into()]
                .spacing(SPACING)
                .align_y(Center)
        ]
        .push(error)
        .spacing(4)
        .into()
    }

    fn range<'a>(&'a self, min: Field, max: Field, unit: Option<&'a str>) -> Element<'a, Message> {
        container(
            row![
                self.number(min, true),
                text("–").size(font::BAR_SIZE),
                self.number(max, true)
            ]
            .push(unit.map(|unit| text(unit).size(font::BAR_SIZE)))
            .spacing(6)
            .align_y(Center),
        )
        .width(RANGE_WIDTH)
        .into()
    }

    fn number(&self, field: Field, enabled: bool) -> Element<'_, Message> {
        text_input(&self.placeholders[index(field)], &self.texts[index(field)])
            .style(style::field)
            .on_input_maybe(enabled.then_some(move |value| Message::Input(field, value)))
            .on_submit(Message::Done)
            .size(font::BAR_SIZE)
            .padding([5, 8])
            .width(NUMBER_WIDTH)
            .into()
    }

    fn forget_error(&mut self, field: Field) {
        if self
            .error
            .as_ref()
            .is_some_and(|(wrong, _)| *wrong == field)
        {
            self.error = None;
        }
    }
}

fn label(name: &str) -> Element<'_, Message> {
    text(name)
        .size(font::BAR_SIZE)
        .style(style::muted)
        .width(LABEL_WIDTH)
        .into()
}

fn action<'a>(
    name: &'a str,
    style: fn(&Theme, button::Status) -> button::Style,
    message: Message,
) -> Element<'a, Message> {
    button(text(name).size(font::BAR_SIZE))
        .style(style)
        .padding([5, 16])
        .on_press(message)
        .into()
}

fn tag_picker<'a>(
    placeholder: &str,
    state: &'a combo_box::State<String>,
    chosen: &'a [String],
    add: fn(String) -> Message,
    remove: fn(String) -> Message,
) -> Element<'a, Message> {
    let chips = row(chosen.iter().map(|tag| {
        button(text!("{tag} ×").size(font::NOTE_SIZE))
            .padding([2, 8])
            .style(style::chip)
            .on_press(remove(tag.clone()))
            .into()
    }))
    .spacing(6)
    .wrap();

    column![
        combo_box(state, placeholder, None, add)
            .input_style(style::field)
            .size(font::BAR_SIZE)
            .padding([5, 8]),
    ]
    .push((!chosen.is_empty()).then_some(chips))
    .spacing(6)
    .width(TAG_WIDTH)
    .into()
}

fn placeholders(defaults: &FilterOptions) -> [String; 6] {
    FIELDS.map(|field| {
        match field {
            Field::MinReviews => defaults.min_reviews.map(format::count),
            Field::MinPositive => defaults
                .min_positive_percent
                .filter(|percent| *percent != 0)
                .map(|percent| percent.to_string()),
            Field::MaxPrice | Field::MinPrice | Field::MinDiscount | Field::MaxReviews => None,
        }
        .unwrap_or_else(|| "any".to_string())
    })
}

fn add(tags: &mut Vec<String>, tag: String) {
    if !tags.contains(&tag) {
        tags.push(tag);
    }
}

fn index(field: Field) -> usize {
    FIELDS
        .iter()
        .position(|kept| *kept == field)
        .expect("every field is listed")
}

fn texts(filters: &FilterOptions) -> [String; 6] {
    FIELDS.map(|field| {
        match field {
            Field::MaxPrice => filters.max_price_cents.map(price_text),
            Field::MinPrice => filters.min_price_cents.map(price_text),
            Field::MinDiscount => filters.min_discount_percent.map(|n| n.to_string()),
            Field::MinReviews => filters.min_reviews.map(|n| n.to_string()),
            Field::MaxReviews => filters.max_reviews.map(|n| n.to_string()),
            Field::MinPositive => filters.min_positive_percent.map(|n| n.to_string()),
        }
        .unwrap_or_default()
    })
}

fn price_text(cents: u32) -> String {
    match cents % 100 {
        0 => (cents / 100).to_string(),
        rest => format!("{}.{rest:02}", cents / 100),
    }
}

fn with_texts(
    filters: &FilterOptions,
    texts: &[String; 6],
) -> Result<FilterOptions, (Field, String)> {
    let parsed = |field, parse: fn(&str) -> Result<u32, String>| {
        optional(texts[index(field)].trim(), parse).map_err(|error| (field, error))
    };
    let parsed_percent =
        |field| optional(texts[index(field)].trim(), percent).map_err(|error| (field, error));
    let min_discount_percent = if filters.only_discounted {
        parsed_percent(Field::MinDiscount)?
    } else {
        None
    };

    Ok(FilterOptions {
        max_price_cents: parsed(Field::MaxPrice, parse_price)?,
        min_price_cents: parsed(Field::MinPrice, parse_price)?,
        min_discount_percent,
        min_reviews: parsed(Field::MinReviews, count)?,
        max_reviews: parsed(Field::MaxReviews, count)?,
        min_positive_percent: parsed_percent(Field::MinPositive)?,
        ..filters.clone()
    })
}

fn optional<T>(text: &str, parse: impl Fn(&str) -> Result<T, String>) -> Result<Option<T>, String> {
    if text.is_empty() {
        Ok(None)
    } else {
        parse(text).map(Some)
    }
}

fn percent(text: &str) -> Result<u8, String> {
    text.trim_end_matches('%')
        .trim()
        .parse()
        .ok()
        .filter(|percent| *percent <= 100)
        .ok_or_else(|| format!("\"{text}\" is not a percent from 0 to 100"))
}

fn count(text: &str) -> Result<u32, String> {
    text.replace([',', '.', ' ', '_'], "")
        .parse()
        .map_err(|_| format!("\"{text}\" is not a number of reviews"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(options: &SearchOptions) -> Panel {
        let defaults = FilterOptions {
            min_reviews: Some(1000),
            min_positive_percent: Some(0),
            without_early_access: Some(true),
            ..FilterOptions::default()
        };
        Panel::new(
            vec!["Metroidvania".to_string()],
            &defaults,
            "€".to_string(),
            options,
        )
    }

    fn type_in(panel: &mut Panel, options: &SearchOptions, field: Field, value: &str) {
        let typed = panel.update(options, Message::Input(field, value.to_string()));
        assert_eq!(typed, Action::Stay);
    }

    fn done(panel: &mut Panel, options: &SearchOptions) -> SearchOptions {
        match panel.update(options, Message::Done) {
            Action::Search(changed) => changed,
            action => panic!("expected a search, got {action:?}"),
        }
    }

    #[test]
    fn the_sources_are_all_tastes_signals_and_the_families() {
        assert_eq!(
            sources(vec!["Puzzle".to_string()]),
            [
                Source::AllTastes,
                Source::Signals,
                Source::Family("Puzzle".to_string())
            ]
        );
    }

    #[test]
    fn a_long_family_name_is_shortened_in_the_menu_only() {
        let long = "Metroidvania · Open World Survival Craft".to_string();
        let family = Source::Family(long.clone());

        assert_eq!(family.to_string(), "Metroidvania · Open World…");
        assert_eq!(
            with_source(&SearchOptions::default(), family).family,
            Some(long)
        );
        assert_eq!(
            Source::Family("Puzzle · Mystery".to_string()).to_string(),
            "Puzzle · Mystery"
        );
    }

    #[test]
    fn choosing_a_source_sets_the_family_or_the_signals() {
        let options = SearchOptions {
            signals_only: true,
            ..SearchOptions::default()
        };

        let family = with_source(&options, Source::Family("Puzzle".to_string()));
        let signals = with_source(&family, Source::Signals);

        assert_eq!(
            (family.family.as_deref(), family.signals_only),
            (Some("Puzzle"), false)
        );
        assert_eq!((signals.family, signals.signals_only), (None, true));
        assert_eq!(source(&family), Source::Family("Puzzle".to_string()));
    }

    #[test]
    fn nothing_applies_before_done() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);

        assert_eq!(panel.update(&options, Message::OnSale(true)), Action::Stay);
        assert_eq!(
            panel.update(&options, Message::AddTag("Metroidvania".to_string())),
            Action::Stay
        );
        let changed = done(&mut panel, &options);

        assert!(changed.filters.only_discounted);
        assert_eq!(changed.filters.tags, ["Metroidvania"]);
    }

    #[test]
    fn done_keeps_the_source_and_the_order_of_the_options() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        panel.update(&options, Message::OnlyBets(true));
        let now = SearchOptions {
            family: Some("Puzzle".to_string()),
            sort: SortBy::Price,
            ..options
        };

        let changed = done(&mut panel, &now);

        assert!(changed.only_bets);
        assert_eq!(changed.family.as_deref(), Some("Puzzle"));
        assert_eq!(changed.sort, SortBy::Price);
    }

    #[test]
    fn done_without_changes_only_closes() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        panel.update(&options, Message::OnSale(true));
        panel.update(&options, Message::OnSale(false));

        assert_eq!(panel.update(&options, Message::Done), Action::Close);
    }

    #[test]
    fn cancel_drops_the_changes() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        panel.update(&options, Message::OnSale(true));
        type_in(&mut panel, &options, Field::MaxPrice, "20");

        assert_eq!(panel.update(&options, Message::Cancel), Action::Close);
        panel.show(&options);

        assert_eq!(panel.filters, options.filters);
        assert_eq!(panel.texts[index(Field::MaxPrice)], "");
    }

    #[test]
    fn typed_values_apply_on_done() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        panel.update(&options, Message::OnSale(true));
        type_in(&mut panel, &options, Field::MaxPrice, "14,99");
        type_in(&mut panel, &options, Field::MinDiscount, "50%");
        type_in(&mut panel, &options, Field::MinReviews, "1,000");

        let changed = done(&mut panel, &options);

        assert_eq!(changed.filters.max_price_cents, Some(1499));
        assert_eq!(changed.filters.min_discount_percent, Some(50));
        assert_eq!(changed.filters.min_reviews, Some(1000));
        assert_eq!(changed.filters.min_price_cents, None);
    }

    #[test]
    fn the_discount_goes_with_on_sale() {
        let mut options = SearchOptions::default();
        options.filters.min_discount_percent = Some(30);
        let mut panel = panel(&options);

        assert!(panel.filters.only_discounted);

        panel.update(&options, Message::OnSale(false));
        let changed = done(&mut panel, &options);

        assert!(!changed.filters.only_discounted);
        assert_eq!(changed.filters.min_discount_percent, None);
        assert_eq!(panel.texts[index(Field::MinDiscount)], "");
    }

    #[test]
    fn early_access_follows_the_config_until_it_differs() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);

        panel.update(&options, Message::EarlyAccess(true));
        assert_eq!(panel.filters.without_early_access, Some(false));

        panel.update(&options, Message::EarlyAccess(false));
        assert_eq!(panel.filters.without_early_access, None);
    }

    #[test]
    fn an_emptied_field_goes_back_to_its_default() {
        let mut options = SearchOptions::default();
        options.filters.max_price_cents = Some(1500);
        let mut panel = panel(&options);
        type_in(&mut panel, &options, Field::MaxPrice, "");

        let changed = done(&mut panel, &options);

        assert_eq!(changed.filters.max_price_cents, None);
    }

    #[test]
    fn a_wrong_value_keeps_the_panel_open_and_points_at_its_field() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        type_in(&mut panel, &options, Field::MinPositive, "120");

        assert_eq!(panel.update(&options, Message::Done), Action::Stay);
        assert_eq!(
            panel.error,
            Some((
                Field::MinPositive,
                "\"120\" is not a percent from 0 to 100".to_string()
            ))
        );
        assert_eq!(panel.texts[index(Field::MinPositive)], "120");

        type_in(&mut panel, &options, Field::MinPositive, "80");
        assert_eq!(panel.error, None);
    }

    #[test]
    fn a_tag_is_added_once_and_removed() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        let tag = || "Metroidvania".to_string();

        panel.update(&options, Message::AddTag(tag()));
        panel.update(&options, Message::AddTag(tag()));
        assert_eq!(panel.filters.tags, [tag()]);

        panel.update(&options, Message::RemoveTag(tag()));
        assert!(panel.filters.tags.is_empty());
    }

    #[test]
    fn reset_empties_the_panel_and_applies_only_after_done() {
        let mut options = SearchOptions {
            family: Some("Puzzle".to_string()),
            only_bets: true,
            ..SearchOptions::default()
        };
        options.filters.without_voted = true;
        options.filters.max_price_cents = Some(1500);
        options.filters.tags = vec!["Metroidvania".to_string()];
        let mut panel = panel(&options);
        type_in(&mut panel, &options, Field::MinPrice, "5");

        assert_eq!(panel.update(&options, Message::Reset), Action::Stay);
        assert!(panel.texts.iter().all(String::is_empty));
        let reset = done(&mut panel, &options);

        let mut expected = SearchOptions {
            family: Some("Puzzle".to_string()),
            ..SearchOptions::default()
        };
        expected.filters.without_voted = true;
        assert_eq!(reset, expected);
    }

    #[test]
    fn the_fields_show_the_current_options() {
        let mut options = SearchOptions::default();
        options.filters.max_price_cents = Some(1500);
        options.filters.min_price_cents = Some(499);

        let panel = panel(&options);

        assert_eq!(panel.texts[index(Field::MaxPrice)], "15");
        assert_eq!(panel.texts[index(Field::MinPrice)], "4.99");
        assert_eq!(panel.texts[index(Field::MinReviews)], "");
    }

    #[test]
    fn the_empty_fields_show_the_values_of_the_config() {
        let panel = panel(&SearchOptions::default());

        assert_eq!(panel.placeholders[index(Field::MinReviews)], "1,000");
        assert_eq!(panel.placeholders[index(Field::MinPositive)], "any");
        assert_eq!(panel.placeholders[index(Field::MaxPrice)], "any");
        assert!(panel.without_early_access);
    }
}
