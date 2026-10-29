use std::path::Path;

use clap::Args;
use gamepalate_core::{
    corrections::Ban,
    profile::{DEFAULT_TAG_LIMIT, ProfileTag, SignalKind, TagCorrection, profile},
    search::SearchInputs,
};
use gamepalate_store::data_files::DataFiles;

use crate::{
    format::{decimal, integer, multiplier},
    names::{family_problem_text, game_label, tag_name},
};

#[derive(Args)]
pub struct ProfileArgs {
    /// Use signals.json and the tags cache as they are, without asking Steam
    #[arg(long)]
    pub offline: bool,

    /// How many tags to show
    #[arg(long, default_value_t = DEFAULT_TAG_LIMIT as u32, value_parser = clap::value_parser!(u32).range(1..))]
    tags: u32,
}

pub fn run(args: &ProfileArgs, files: &DataFiles, data_dir: &Path) -> anyhow::Result<()> {
    let inputs = files.search_inputs(&[]);
    let Some(mut profile) = profile(&inputs, args.tags as usize) else {
        println!("No game above the hours threshold.");
        return Ok(());
    };
    profile
        .corrected_tags
        .sort_by(|a, b| b.weight.total_cmp(&a.weight));

    println!("Top tags of your profile:");
    for tag in &profile.tags {
        print_tag(tag, &inputs);
    }

    if !profile.corrected_tags.is_empty() || !profile.corrected_games.is_empty() {
        println!();
        println!("Effect of the corrections:");
        for tag in &profile.corrected_tags {
            let label = tag
                .correction
                .as_ref()
                .map(correction_label)
                .unwrap_or_default();
            let effect = match tag.correction.as_ref().and_then(|c| c.multiplier) {
                Some(_) => format!("{} -> {}", decimal(tag.weight_before), decimal(tag.weight)),
                None => decimal(tag.weight),
            };
            println!(
                "  {} (#{})  {label}  {effect}",
                tag_name(tag.id, inputs.catalog),
                tag.id.0
            );
        }
        for game in &profile.corrected_games {
            let effect = if game.in_effect {
                format!(
                    "{} -> {}",
                    decimal(game.weight_before),
                    decimal(game.weight)
                )
            } else {
                "no effect".to_string()
            };
            println!(
                "  {}  {}  {effect}",
                game_label(game.id, inputs.catalog, inputs.off_catalog),
                multiplier(game.multiplier)
            );
        }
    }

    let families_path = data_dir.join("families.toml");
    if profile.families.is_empty() && profile.family_problems.is_empty() {
        println!();
        println!(
            "No families: write them by hand in {}",
            families_path.display()
        );
    }
    for family in &profile.families {
        println!();
        println!("Family \"{}\":", family.name);
        println!("  Games:");
        for &game in &family.games {
            println!(
                "    {}",
                game_label(game, inputs.catalog, inputs.off_catalog)
            );
        }
        println!("  Top tags:");
        for &(tag, weight) in &family.tags {
            println!(
                "    {} (#{})  {}",
                tag_name(tag, inputs.catalog),
                tag.0,
                decimal(weight)
            );
        }
    }

    if !profile.families.is_empty() || profile.kept_aside > 0 {
        println!();
        println!("Held-out games: {}.", integer(profile.kept_aside as u64));
    }

    if !profile.family_problems.is_empty() {
        println!();
        println!("Problems in {}:", families_path.display());
        for problem in &profile.family_problems {
            println!("  - {}", family_problem_text(problem, &inputs));
        }
    }

    Ok(())
}

fn print_tag(tag: &ProfileTag, inputs: &SearchInputs) {
    let note = tag
        .correction
        .as_ref()
        .map(|correction| format!("  [{}]", correction_label(correction)))
        .unwrap_or_default();
    println!(
        "  {} (#{})  {}{note}",
        tag_name(tag.id, inputs.catalog),
        tag.id.0,
        decimal(tag.weight)
    );
    for game in &tag.games {
        let kinds: Vec<&str> = game.kinds.iter().map(|&kind| kind_name(kind)).collect();
        println!(
            "      {}  {:.0}%  ({})",
            game_label(game.id, inputs.catalog, inputs.off_catalog),
            game.share * 100.0,
            kinds.join(", ")
        );
    }
}

fn correction_label(correction: &TagCorrection) -> String {
    let mut parts = Vec::new();
    match correction.banned {
        Some(Ban::Everywhere) => parts.push("banned".to_string()),
        Some(Ban::TopTags(n)) => parts.push(format!("banned within the first {n} tags")),
        None => {}
    }
    if let Some(value) = correction.multiplier {
        parts.push(multiplier(value));
    }
    parts.join(", ")
}

fn kind_name(kind: SignalKind) -> &'static str {
    match kind {
        SignalKind::Library => "library",
        SignalKind::Recent => "recent",
        SignalKind::Wishlist => "wishlist",
        SignalKind::Followed => "followed",
        SignalKind::WouldTry => "would try",
    }
}
