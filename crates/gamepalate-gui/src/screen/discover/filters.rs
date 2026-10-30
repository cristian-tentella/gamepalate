use std::fmt;

use gamepalate_app::{FilterOptions, SearchOptions, SortBy, parse_price};
use iced::{
    Bottom, Element, Fill, padding,
    widget::{button, checkbox, column, combo_box, container, pick_list, row, text, text_input},
};

use crate::{font, style};

const FIELD_WIDTH: f32 = 200.0;

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
            Source::Family(name) => formatter.write_str(name),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sort(SortBy);

impl fmt::Display for Sort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.0 {
            SortBy::Relevance => "Relevance",
            SortBy::Discount => "Discount",
            SortBy::Price => "Price",
        })
    }
}

const SORTS: [Sort; 3] = [
    Sort(SortBy::Relevance),
    Sort(SortBy::Discount),
    Sort(SortBy::Price),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EarlyAccess(Option<bool>);

impl fmt::Display for EarlyAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.0 {
            None => "As in the config",
            Some(false) => "Included",
            Some(true) => "Left out",
        })
    }
}

const EARLY_ACCESS: [EarlyAccess; 3] = [
    EarlyAccess(None),
    EarlyAccess(Some(false)),
    EarlyAccess(Some(true)),
];

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
    Source(Source),
    OnSale(bool),
    OnlyBets(bool),
    Sort(Sort),
    EarlyAccess(EarlyAccess),
    Input(Field, String),
    Submit,
    AddTag(String),
    RemoveTag(String),
    AddWithoutTag(String),
    RemoveWithoutTag(String),
    Reset,
}

pub struct Panel {
    sources: Vec<Source>,
    drafts: [String; 6],
    tags: combo_box::State<String>,
    without_tags: combo_box::State<String>,
}

impl Panel {
    pub fn new(families: Vec<String>, tags: Vec<String>, options: &SearchOptions) -> Panel {
        let sources = [Source::AllTastes, Source::Signals]
            .into_iter()
            .chain(families.into_iter().map(Source::Family))
            .collect();

        Panel {
            sources,
            drafts: drafts(options),
            tags: combo_box::State::new(tags.clone()),
            without_tags: combo_box::State::new(tags),
        }
    }

    pub fn show(&mut self, options: &SearchOptions) {
        self.drafts = drafts(options);
    }

    pub fn update(
        &mut self,
        options: &SearchOptions,
        message: Message,
    ) -> Result<Option<SearchOptions>, String> {
        let mut changed = options.clone();
        let filters = &mut changed.filters;
        match message {
            Message::Source(source) => {
                (changed.family, changed.signals_only) = match source {
                    Source::AllTastes => (None, false),
                    Source::Signals => (None, true),
                    Source::Family(name) => (Some(name), false),
                };
            }
            Message::OnSale(on) => filters.only_discounted = on,
            Message::OnlyBets(on) => changed.only_bets = on,
            Message::Sort(Sort(sort)) => changed.sort = sort,
            Message::EarlyAccess(EarlyAccess(without)) => filters.without_early_access = without,
            Message::Input(field, value) => {
                self.drafts[index(field)] = value;
                return Ok(None);
            }
            Message::Submit => changed = with_drafts(options, &self.drafts)?,
            Message::AddTag(tag) => add(&mut filters.tags, tag),
            Message::RemoveTag(tag) => filters.tags.retain(|kept| *kept != tag),
            Message::AddWithoutTag(tag) => add(&mut filters.without_tags, tag),
            Message::RemoveWithoutTag(tag) => filters.without_tags.retain(|kept| *kept != tag),
            Message::Reset => {
                changed = defaults(options);
                self.drafts = drafts(&changed);
            }
        }

        Ok((changed != *options).then_some(changed))
    }

    pub fn view<'a>(&'a self, options: &'a SearchOptions) -> Element<'a, Message> {
        let filters = &options.filters;

        let frequent = row![
            labeled(
                "From",
                pick_list(
                    self.sources.as_slice(),
                    Some(source(options)),
                    Message::Source
                )
                .text_size(font::BAR_SIZE)
                .width(Fill),
            ),
            labeled("Max price", self.input(Field::MaxPrice, "any"),),
            checked(
                checkbox(filters.only_discounted)
                    .label("On sale")
                    .text_size(font::BAR_SIZE)
                    .on_toggle(Message::OnSale),
            ),
        ]
        .spacing(16)
        .align_y(Bottom);

        let more = row![
            labeled(
                "Sort",
                pick_list(SORTS, Some(Sort(options.sort)), Message::Sort)
                    .text_size(font::BAR_SIZE)
                    .width(Fill),
            ),
            labeled("Min discount %", self.input(Field::MinDiscount, "any")),
            labeled("Min price", self.input(Field::MinPrice, "any")),
        ]
        .spacing(16);

        let reviews = row![
            labeled(
                "Min reviews",
                self.input(Field::MinReviews, "as in the config")
            ),
            labeled("Max reviews", self.input(Field::MaxReviews, "any")),
            labeled(
                "Min % positive",
                self.input(Field::MinPositive, "as in the config")
            ),
        ]
        .spacing(16);

        let tags = row![
            tag_picker(
                "With tags",
                &self.tags,
                &filters.tags,
                Message::AddTag,
                Message::RemoveTag
            ),
            tag_picker(
                "Without tags",
                &self.without_tags,
                &filters.without_tags,
                Message::AddWithoutTag,
                Message::RemoveWithoutTag,
            ),
        ]
        .spacing(16);

        let last = row![
            labeled(
                "Early access",
                pick_list(
                    EARLY_ACCESS,
                    Some(EarlyAccess(filters.without_early_access)),
                    Message::EarlyAccess
                )
                .text_size(font::BAR_SIZE)
                .width(Fill),
            ),
            checked(
                checkbox(options.only_bets)
                    .label("Bets only")
                    .text_size(font::BAR_SIZE)
                    .on_toggle(Message::OnlyBets),
            ),
            container(
                button(text("Reset").size(font::BAR_SIZE))
                    .padding([5, 16])
                    .on_press(Message::Reset),
            )
            .align_right(FIELD_WIDTH),
        ]
        .spacing(16)
        .align_y(Bottom);

        column![
            frequent,
            more,
            reviews,
            tags,
            last,
            text("Prices and numbers apply with Enter · Esc closes")
                .size(font::NOTE_SIZE)
                .style(style::muted),
        ]
        .spacing(12)
        .into()
    }

    fn input<'a>(&'a self, field: Field, placeholder: &str) -> Element<'a, Message> {
        text_input(placeholder, &self.drafts[index(field)])
            .on_input(move |value| Message::Input(field, value))
            .on_submit(Message::Submit)
            .size(font::BAR_SIZE)
            .padding([5, 8])
            .into()
    }
}

fn labeled<'a>(label: &'a str, widget: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![
        text(label).size(font::NOTE_SIZE).style(style::muted),
        widget.into()
    ]
    .spacing(4)
    .width(FIELD_WIDTH)
    .into()
}

fn checked<'a>(widget: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(widget)
        .padding(padding::bottom(6))
        .width(FIELD_WIDTH)
        .into()
}

fn tag_picker<'a>(
    label: &'a str,
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
        text(label).size(font::NOTE_SIZE).style(style::muted),
        combo_box(state, "type a tag", None, add)
            .size(font::BAR_SIZE)
            .padding([5, 8]),
        chips,
    ]
    .spacing(4)
    .width(FIELD_WIDTH * 1.5 + 8.0)
    .into()
}

fn defaults(options: &SearchOptions) -> SearchOptions {
    SearchOptions {
        limit: options.limit,
        filters: FilterOptions {
            without_voted: options.filters.without_voted,
            ..FilterOptions::default()
        },
        ..SearchOptions::default()
    }
}

fn source(options: &SearchOptions) -> Source {
    match (&options.family, options.signals_only) {
        (Some(name), _) => Source::Family(name.clone()),
        (None, true) => Source::Signals,
        (None, false) => Source::AllTastes,
    }
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

fn drafts(options: &SearchOptions) -> [String; 6] {
    let filters = &options.filters;
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

fn draft(drafts: &[String; 6], field: Field) -> &str {
    drafts[index(field)].trim()
}

fn with_drafts(options: &SearchOptions, drafts: &[String; 6]) -> Result<SearchOptions, String> {
    let filters = FilterOptions {
        max_price_cents: optional(draft(drafts, Field::MaxPrice), parse_price)?,
        min_price_cents: optional(draft(drafts, Field::MinPrice), parse_price)?,
        min_discount_percent: optional(draft(drafts, Field::MinDiscount), percent)?,
        min_reviews: optional(draft(drafts, Field::MinReviews), count)?,
        max_reviews: optional(draft(drafts, Field::MaxReviews), count)?,
        min_positive_percent: optional(draft(drafts, Field::MinPositive), percent)?,
        ..options.filters.clone()
    };

    Ok(SearchOptions {
        filters,
        ..options.clone()
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
        Panel::new(
            vec!["Puzzle".to_string()],
            vec!["Metroidvania".to_string()],
            options,
        )
    }

    fn type_in(panel: &mut Panel, options: &SearchOptions, field: Field, value: &str) {
        let typed = panel.update(options, Message::Input(field, value.to_string()));
        assert_eq!(typed, Ok(None));
    }

    #[test]
    fn the_sources_are_all_tastes_signals_and_the_families() {
        let panel = panel(&SearchOptions::default());

        assert_eq!(
            panel.sources,
            [
                Source::AllTastes,
                Source::Signals,
                Source::Family("Puzzle".to_string())
            ]
        );
    }

    #[test]
    fn choosing_a_source_sets_the_family_or_the_signals() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);

        let family = panel
            .update(
                &options,
                Message::Source(Source::Family("Puzzle".to_string())),
            )
            .unwrap()
            .unwrap();
        let signals = panel
            .update(&family, Message::Source(Source::Signals))
            .unwrap()
            .unwrap();

        assert_eq!(family.family.as_deref(), Some("Puzzle"));
        assert_eq!((signals.family, signals.signals_only), (None, true));
        assert_eq!(source(&family), Source::Family("Puzzle".to_string()));
    }

    #[test]
    fn the_same_choice_changes_nothing() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);

        assert_eq!(panel.update(&options, Message::OnSale(false)), Ok(None));
        assert_eq!(
            panel.update(&options, Message::Sort(Sort(SortBy::Relevance))),
            Ok(None)
        );
    }

    #[test]
    fn typed_values_apply_on_submit() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        type_in(&mut panel, &options, Field::MaxPrice, "14,99");
        type_in(&mut panel, &options, Field::MinDiscount, "50%");
        type_in(&mut panel, &options, Field::MinReviews, "1,000");

        let changed = panel.update(&options, Message::Submit).unwrap().unwrap();

        assert_eq!(changed.filters.max_price_cents, Some(1499));
        assert_eq!(changed.filters.min_discount_percent, Some(50));
        assert_eq!(changed.filters.min_reviews, Some(1000));
        assert_eq!(changed.filters.min_price_cents, None);
    }

    #[test]
    fn an_emptied_field_goes_back_to_its_default() {
        let mut options = SearchOptions::default();
        options.filters.max_price_cents = Some(1500);
        let mut panel = panel(&options);
        type_in(&mut panel, &options, Field::MaxPrice, "");

        let changed = panel.update(&options, Message::Submit).unwrap().unwrap();

        assert_eq!(changed.filters.max_price_cents, None);
    }

    #[test]
    fn a_wrong_value_is_an_error() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        type_in(&mut panel, &options, Field::MinPositive, "120");

        assert_eq!(
            panel.update(&options, Message::Submit),
            Err("\"120\" is not a percent from 0 to 100".to_string())
        );
    }

    #[test]
    fn a_tag_is_added_once_and_removed() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
        let tag = || "Metroidvania".to_string();

        let added = panel
            .update(&options, Message::AddTag(tag()))
            .unwrap()
            .unwrap();
        let again = panel.update(&added, Message::AddTag(tag())).unwrap();
        let removed = panel
            .update(&added, Message::RemoveTag(tag()))
            .unwrap()
            .unwrap();

        assert_eq!(added.filters.tags, [tag()]);
        assert_eq!(again, None);
        assert!(removed.filters.tags.is_empty());
    }

    #[test]
    fn reset_goes_back_to_the_defaults_and_keeps_out_the_voted_games() {
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

        let reset = panel.update(&options, Message::Reset).unwrap().unwrap();

        let mut expected = SearchOptions::default();
        expected.filters.without_voted = true;
        assert_eq!(reset, expected);
        assert!(panel.drafts.iter().all(String::is_empty));
        assert_eq!(panel.update(&reset, Message::Reset), Ok(None));
    }

    #[test]
    fn the_fields_show_the_current_options() {
        let mut options = SearchOptions::default();
        options.filters.max_price_cents = Some(1500);
        options.filters.min_price_cents = Some(499);

        let panel = panel(&options);

        assert_eq!(panel.drafts[index(Field::MaxPrice)], "15");
        assert_eq!(panel.drafts[index(Field::MinPrice)], "4.99");
        assert_eq!(panel.drafts[index(Field::MinReviews)], "");
    }
}
