use std::path::Path;

use clap::{Args, Subcommand};
use gamepalate_app::families::{AutomaticFamilies, create_automatic_families};

use crate::{names::game_label, signals};

#[derive(Args)]
pub struct FamiliesArgs {
    #[command(subcommand)]
    command: FamiliesCommand,
}

#[derive(Subcommand)]
enum FamiliesCommand {
    /// Make families.toml from the games you play, if there is none yet
    Create(CreateArgs),
}

#[derive(Args)]
struct CreateArgs {
    /// Use signals.json and the tags cache as they are, without asking Steam
    #[arg(long)]
    offline: bool,
}

pub fn run(args: &FamiliesArgs, data_dir: &Path) -> anyhow::Result<()> {
    match &args.command {
        FamiliesCommand::Create(create) => {
            let files = signals::load(data_dir, create.offline)?;
            let path = data_dir.join("families.toml");
            let date = jiff::Zoned::now().date().to_string();
            let outcome = create_automatic_families(&path, &files.search_inputs(&[]), &date)?;

            let AutomaticFamilies::Created(created) = &outcome else {
                println!("No families made: {}.", not_made_reason(&outcome));
                return Ok(());
            };
            let families = &created.families.families;
            let noun = if families.len() == 1 {
                "family"
            } else {
                "families"
            };
            println!(
                "Made {} {noun} from the games you play, in {}:",
                families.len(),
                path.display()
            );
            for family in families {
                println!();
                println!("Family \"{}\" (automatic):", family.name);
                for &game in &family.games {
                    println!("  {}", game_label(game, &files.catalog, &files.off_catalog));
                }
            }
        }
    }
    Ok(())
}

fn not_made_reason(outcome: &AutomaticFamilies) -> &'static str {
    match outcome {
        AutomaticFamilies::Created(_) => "",
        AutomaticFamilies::FileExists => "families.toml already exists",
        AutomaticFamilies::Off => "automatic families are off in config.toml",
        AutomaticFamilies::TooFewGames => "fewer than 6 played games",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_why_no_families_were_made() {
        assert_eq!(
            not_made_reason(&AutomaticFamilies::FileExists),
            "families.toml already exists"
        );
        assert_eq!(
            not_made_reason(&AutomaticFamilies::Off),
            "automatic families are off in config.toml"
        );
        assert_eq!(
            not_made_reason(&AutomaticFamilies::TooFewGames),
            "fewer than 6 played games"
        );
    }
}
