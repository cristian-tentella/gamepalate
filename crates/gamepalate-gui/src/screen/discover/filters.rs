use std::fmt;

use gamepalate_app::{FilterOptions, SearchOptions, SortBy, parse_price};
use iced::{
    Bottom, Center, Element, Fill,
    widget::{button, checkbox, column, combo_box, pick_list, row, space, text, text_input},
};

use crate::{font, format, style};

const FIELD_WIDTH: f32 = 200.0;
const FIELD_SPACING: f32 = 16.0;

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
pub struct EarlyAccess {
    without: Option<bool>,
    default: bool,
}

impl fmt::Display for EarlyAccess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match (self.without, self.default) {
            (None, false) => "Default (included)",
            (None, true) => "Default (left out)",
            (Some(false), _) => "Included",
            (Some(true), _) => "Left out",
        })
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
    Source(Source),
    OnSale(bool),
    OnlyBets(bool),
    Sort(Sort),
    EarlyAccess(EarlyAccess),
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
    sources: Vec<Source>,
    early_access: [EarlyAccess; 3],
    placeholders: [String; 6],
    currency: String,
    draft: SearchOptions,
    texts: [String; 6],
    error: Option<(Field, String)>,
    tags: combo_box::State<String>,
    without_tags: combo_box::State<String>,
}

impl Panel {
    pub fn new(
        families: Vec<String>,
        tags: Vec<String>,
        defaults: &FilterOptions,
        currency: String,
        options: &SearchOptions,
    ) -> Panel {
        let sources = [Source::AllTastes, Source::Signals]
            .into_iter()
            .chain(families.into_iter().map(Source::Family))
            .collect();
        let default = defaults.without_early_access.unwrap_or(false);

        Panel {
            sources,
            early_access: [None, Some(false), Some(true)]
                .map(|without| EarlyAccess { without, default }),
            placeholders: placeholders(defaults),
            currency,
            draft: options.clone(),
            texts: texts(options),
            error: None,
            tags: combo_box::State::new(tags.clone()),
            without_tags: combo_box::State::new(tags),
        }
    }

    pub fn show(&mut self, options: &SearchOptions) {
        self.draft = options.clone();
        self.texts = texts(options);
        self.error = None;
    }

    pub fn update(&mut self, options: &SearchOptions, message: Message) -> Action {
        let draft = &mut self.draft;
        match message {
            Message::Source(source) => {
                (draft.family, draft.signals_only) = match source {
                    Source::AllTastes => (None, false),
                    Source::Signals => (None, true),
                    Source::Family(name) => (Some(name), false),
                };
            }
            Message::OnSale(on) => draft.filters.only_discounted = on,
            Message::OnlyBets(on) => draft.only_bets = on,
            Message::Sort(Sort(sort)) => draft.sort = sort,
            Message::EarlyAccess(choice) => draft.filters.without_early_access = choice.without,
            Message::Input(field, value) => {
                self.texts[index(field)] = value;
                if self
                    .error
                    .as_ref()
                    .is_some_and(|(wrong, _)| *wrong == field)
                {
                    self.error = None;
                }
            }
            Message::AddTag(tag) => add(&mut draft.filters.tags, tag),
            Message::RemoveTag(tag) => draft.filters.tags.retain(|kept| *kept != tag),
            Message::AddWithoutTag(tag) => add(&mut draft.filters.without_tags, tag),
            Message::RemoveWithoutTag(tag) => {
                draft.filters.without_tags.retain(|kept| *kept != tag)
            }
            Message::Reset => {
                *draft = defaults(draft);
                self.texts = texts(draft);
                self.error = None;
            }
            Message::Cancel => return Action::Close,
            Message::Done => match with_texts(draft, &self.texts) {
                Ok(changed) if changed == *options => return Action::Close,
                Ok(changed) => return Action::Search(changed),
                Err(error) => self.error = Some(error),
            },
        }

        Action::Stay
    }

    pub fn view(&self) -> Element<'_, Message> {
        let draft = &self.draft;
        let filters = &draft.filters;

        let games = section(
            "Games",
            column![
                row![
                    labeled(
                        "From",
                        pick_list(
                            self.sources.as_slice(),
                            Some(source(draft)),
                            Message::Source
                        )
                        .text_size(font::BAR_SIZE)
                        .width(Fill),
                    ),
                    labeled(
                        "Sort",
                        pick_list(SORTS, Some(Sort(draft.sort)), Message::Sort)
                            .text_size(font::BAR_SIZE)
                            .width(Fill),
                    ),
                    labeled(
                        "Early access",
                        pick_list(
                            self.early_access,
                            self.early_access
                                .iter()
                                .copied()
                                .find(|choice| choice.without == filters.without_early_access),
                            Message::EarlyAccess
                        )
                        .text_size(font::BAR_SIZE)
                        .width(Fill),
                    ),
                ]
                .spacing(FIELD_SPACING),
                row![
                    checkbox(draft.only_bets)
                        .label("Bets only")
                        .text_size(font::BAR_SIZE)
                        .on_toggle(Message::OnlyBets),
                    text("Games your votes would skip")
                        .size(font::NOTE_SIZE)
                        .style(style::muted),
                ]
                .spacing(8)
                .align_y(Center),
            ]
            .spacing(12),
        );

        let price = section(
            &format!("Price ({})", self.currency),
            column![
                row![
                    self.field("Min price", Field::MinPrice),
                    self.field("Max price", Field::MaxPrice),
                    self.field("Min discount %", Field::MinDiscount),
                ]
                .spacing(FIELD_SPACING),
                checkbox(filters.only_discounted)
                    .label("On sale only")
                    .text_size(font::BAR_SIZE)
                    .on_toggle(Message::OnSale),
            ]
            .spacing(12),
        );

        let reviews = section(
            "Reviews",
            row![
                self.field("Min reviews", Field::MinReviews),
                self.field("Max reviews", Field::MaxReviews),
                self.field("Min % positive", Field::MinPositive),
            ]
            .spacing(FIELD_SPACING),
        );

        let tags = section(
            "Tags",
            row![
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
            .spacing(FIELD_SPACING),
        );

        let actions = row![
            action("Reset", style::secondary, Message::Reset),
            space().width(Fill),
            action("Cancel", style::secondary, Message::Cancel),
            action("Done", style::primary, Message::Done),
        ]
        .spacing(8)
        .align_y(Bottom);

        column![games, price, reviews, tags, actions]
            .spacing(20)
            .width(FIELD_WIDTH * 3.0 + FIELD_SPACING * 2.0)
            .into()
    }

    fn field<'a>(&'a self, label: &'a str, field: Field) -> Element<'a, Message> {
        let input = text_input(&self.placeholders[index(field)], &self.texts[index(field)])
            .style(style::field)
            .on_input(move |value| Message::Input(field, value))
            .on_submit(Message::Done)
            .size(font::BAR_SIZE)
            .padding([5, 8]);
        let error = self
            .error
            .as_ref()
            .filter(|(wrong, _)| *wrong == field)
            .map(|(_, error)| text(error).size(font::NOTE_SIZE).style(text::danger));

        column![labeled(label, input)]
            .push(error)
            .spacing(4)
            .width(FIELD_WIDTH)
            .into()
    }
}

fn section<'a>(title: &str, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    column![
        text(title.to_string())
            .size(font::BAR_SIZE)
            .font(font::SEMIBOLD),
        content.into()
    ]
    .spacing(8)
    .into()
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

fn action<'a>(
    label: &'a str,
    style: fn(&iced::Theme, button::Status) -> button::Style,
    message: Message,
) -> Element<'a, Message> {
    button(text(label).size(font::BAR_SIZE))
        .style(style)
        .padding([5, 16])
        .on_press(message)
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
    .width(FIELD_WIDTH * 1.5 + FIELD_SPACING / 2.0)
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

fn texts(options: &SearchOptions) -> [String; 6] {
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

fn with_texts(
    options: &SearchOptions,
    texts: &[String; 6],
) -> Result<SearchOptions, (Field, String)> {
    let parsed = |field, parse: fn(&str) -> Result<u32, String>| {
        optional(texts[index(field)].trim(), parse).map_err(|error| (field, error))
    };
    let parsed_percent =
        |field| optional(texts[index(field)].trim(), percent).map_err(|error| (field, error));
    let filters = FilterOptions {
        max_price_cents: parsed(Field::MaxPrice, parse_price)?,
        min_price_cents: parsed(Field::MinPrice, parse_price)?,
        min_discount_percent: parsed_percent(Field::MinDiscount)?,
        min_reviews: parsed(Field::MinReviews, count)?,
        max_reviews: parsed(Field::MaxReviews, count)?,
        min_positive_percent: parsed_percent(Field::MinPositive)?,
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
        let defaults = FilterOptions {
            min_reviews: Some(1000),
            min_positive_percent: Some(0),
            without_early_access: Some(true),
            ..FilterOptions::default()
        };
        Panel::new(
            vec!["Puzzle".to_string()],
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

        panel.update(
            &options,
            Message::Source(Source::Family("Puzzle".to_string())),
        );
        let family = done(&mut panel, &options);
        panel.update(&family, Message::Source(Source::Signals));
        let signals = done(&mut panel, &family);

        assert_eq!(family.family.as_deref(), Some("Puzzle"));
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

        assert_eq!(panel.draft, options);
        assert_eq!(panel.texts[index(Field::MaxPrice)], "");
    }

    #[test]
    fn typed_values_apply_on_done() {
        let options = SearchOptions::default();
        let mut panel = panel(&options);
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
        assert_eq!(panel.draft.filters.tags, [tag()]);

        panel.update(&options, Message::RemoveTag(tag()));
        assert!(panel.draft.filters.tags.is_empty());
    }

    #[test]
    fn reset_goes_back_to_the_defaults_only_after_done() {
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

        let mut expected = SearchOptions::default();
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
        assert_eq!(panel.early_access[0].to_string(), "Default (left out)");
    }
}
