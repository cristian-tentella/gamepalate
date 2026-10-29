use std::path::Path;

use clap::Args;
use comfy_table::{CellAlignment, ContentArrangement, Table, presets::UTF8_FULL_CONDENSED};
use gamepalate_core::{
    config::Config,
    evaluation::{EvaluatedGame, GroupSummary, evaluate},
    game_vector::game_vectors,
};
use gamepalate_store::data_files::DataFiles;

use crate::{
    format::{decimal, integer, parse_number},
    names::check_families,
};

#[derive(Args)]
pub struct EvaluateArgs {
    /// Use signals.json and the tags cache as they are, without asking Steam
    #[arg(long)]
    pub offline: bool,

    /// Weight of the quality in the relevance, from 0 to 1
    #[arg(long, value_parser = parse_unit)]
    quality_weight: Option<f64>,

    /// Games with fewer reviews have no position
    #[arg(long)]
    min_reviews: Option<u32>,

    /// Games played less than this many hours are left out of the signals
    #[arg(long, value_parser = parse_hours)]
    min_hours: Option<f64>,

    /// Hours above this cap count as the cap
    #[arg(long, value_parser = parse_hours)]
    max_hours: Option<f64>,

    /// Multiplier of the hours played in the last two weeks
    #[arg(long, value_parser = parse_hours)]
    recent_hours_multiplier: Option<f64>,
}

impl EvaluateArgs {
    fn changes_config(&self) -> bool {
        self.quality_weight.is_some()
            || self.min_reviews.is_some()
            || self.min_hours.is_some()
            || self.max_hours.is_some()
            || self.recent_hours_multiplier.is_some()
    }

    pub fn apply(&self, config: &Config) -> Config {
        let mut config = *config;
        if let Some(value) = self.quality_weight {
            config.relevance.quality_weight = value;
        }
        if let Some(value) = self.min_reviews {
            config.relevance.min_reviews = value;
        }
        if let Some(value) = self.min_hours {
            config.signals.min_hours = value;
        }
        if let Some(value) = self.max_hours {
            config.signals.max_hours = value;
        }
        if let Some(value) = self.recent_hours_multiplier {
            config.signals.recent_hours_multiplier = value;
        }
        config
    }
}

fn parse_unit(text: &str) -> Result<f64, String> {
    let value = parse_number(text)?;
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{value} is not between 0 and 1"))
    }
}

fn parse_hours(text: &str) -> Result<f64, String> {
    let value = parse_number(text)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(format!("{value} is negative"))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Change {
    Better,
    Worse,
    Similar,
}

fn change(position: Option<usize>, base: Option<usize>) -> Change {
    match (position, base) {
        (Some(position), Some(base)) if position * 2 <= base => Change::Better,
        (Some(position), Some(base)) if position >= base * 2 => Change::Worse,
        _ => Change::Similar,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Group {
    KeptAside,
    Guardrail,
    Families,
}

pub fn run(
    args: &EvaluateArgs,
    files: &DataFiles,
    config: &Config,
    data_dir: &Path,
) -> anyhow::Result<()> {
    let vectors = game_vectors(&files.catalog);
    let inputs = files.search_inputs(&vectors);
    check_families(files, &inputs, data_dir)?;

    let mut changed_inputs = inputs;
    changed_inputs.config = config;

    let evaluation = evaluate(&changed_inputs);
    let base = args.changes_config().then(|| evaluate(&inputs));

    println!(
        "Quality weight {}, min reviews {}, hours from {} to {}, recent hours ×{}.",
        config.relevance.quality_weight,
        integer(u64::from(config.relevance.min_reviews)),
        config.signals.min_hours,
        config.signals.max_hours,
        config.signals.recent_hours_multiplier
    );
    if base.is_some() {
        println!(
            "Compared with the config of {}.",
            data_dir.join("config.toml").display()
        );
    }
    println!(
        "Catalog: {} games. The three groups are read on their own: their numbers do not add up.",
        integer(evaluation.catalog_size as u64)
    );

    let families_path = data_dir.join("families.toml");
    let groups = [
        (
            Group::KeptAside,
            "Held-out games",
            &evaluation.kept_aside,
            base.as_ref().map(|base| &base.kept_aside),
            format!(
                "No held-out games: list them by hand in [held_out] of {}",
                families_path.display()
            ),
        ),
        (
            Group::Guardrail,
            "Guardrail: the most played games outside the families",
            &evaluation.guardrail,
            base.as_ref().map(|base| &base.guardrail),
            "No guardrail games: no played game in the catalog is outside the families and the held-out games".to_string(),
        ),
        (
            Group::Families,
            "Family games",
            &evaluation.families,
            base.as_ref().map(|base| &base.families),
            format!("No families: write them by hand in {}", families_path.display()),
        ),
    ];

    for (group, title, summary, base, empty) in groups {
        println!();
        println!("{title}:");
        if summary.games.is_empty() {
            println!("{empty}.");
            continue;
        }
        print_group(group, summary, base);
    }

    Ok(())
}

fn print_group(group: Group, summary: &GroupSummary, base: Option<&GroupSummary>) {
    let base_position = |game: &EvaluatedGame| {
        base.and_then(|base| base.games.iter().find(|other| other.id == game.id))
            .and_then(|other| other.position)
    };

    let mut header = vec!["Position"];
    if base.is_some() {
        header.extend(["File config", "Change"]);
    }
    header.push("Game");
    match group {
        Group::Guardrail => header.push("Hours"),
        Group::Families => header.push("Families"),
        Group::KeptAside => {}
    }

    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(header);

    let mut counts = [0usize; 3];
    for game in &summary.games {
        let mut row = vec![position_text(game.position)];
        if base.is_some() {
            let before = base_position(game);
            let change = change(game.position, before);
            counts[change as usize] += 1;
            row.push(position_text(before));
            row.push(
                match change {
                    Change::Better => "better",
                    Change::Worse => "worse",
                    Change::Similar => "",
                }
                .to_string(),
            );
        }
        row.push(game.name.clone());
        match group {
            Group::Guardrail => row.push(game.effective_hours.map_or("-".to_string(), decimal)),
            Group::Families => row.push(game.families.join(", ")),
            Group::KeptAside => {}
        }
        table.add_row(row);
    }

    let mut right_aligned = vec![0];
    if base.is_some() {
        right_aligned.push(1);
    }
    if group == Group::Guardrail {
        right_aligned.push(table.column_count() - 1);
    }
    for column in right_aligned {
        if let Some(column) = table.column_mut(column) {
            column.set_cell_alignment(CellAlignment::Right);
        }
    }
    println!("{table}");

    println!(
        "Top 100: {}. Top 1000: {}. Median: {}. Geometric mean: {}. Without position: {}.",
        with_base(summary, base, |s| s.in_top.to_string()),
        with_base(summary, base, |s| s.in_top_wide.to_string()),
        with_base(summary, base, |s| average_text(s.median)),
        with_base(summary, base, |s| average_text(s.geometric_mean)),
        with_base(summary, base, |s| s.without_position.to_string()),
    );
    if base.is_some() {
        println!(
            "Better: {}. Worse: {}. Similar: {}.",
            counts[Change::Better as usize],
            counts[Change::Worse as usize],
            counts[Change::Similar as usize]
        );
    }
}

fn with_base(
    summary: &GroupSummary,
    base: Option<&GroupSummary>,
    value: impl Fn(&GroupSummary) -> String,
) -> String {
    match base {
        Some(base) => format!("{} (file {})", value(summary), value(base)),
        None => value(summary),
    }
}

fn position_text(position: Option<usize>) -> String {
    position.map_or("-".to_string(), |position| integer(position as u64))
}

fn average_text(value: Option<f64>) -> String {
    value.map_or("-".to_string(), decimal)
}
