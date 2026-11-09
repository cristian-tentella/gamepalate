use std::{collections::BTreeMap, path::Path};

use clap::{Args, Subcommand};
use gamepalate_app::families::{
    AutomaticFamilies, MAX_FAMILIES, RemadeFamilies, create_automatic_families,
    remake_automatic_families,
};
use gamepalate_core::{
    catalog::Catalog, families::Family, ids::AppId, off_catalog::OffCatalogGame,
};

use crate::{names::game_label, signals};

#[derive(Args)]
pub struct FamiliesArgs {
    #[command(subcommand)]
    command: FamiliesCommand,
}

#[derive(Subcommand)]
enum FamiliesCommand {
    /// Make families.toml from the games you play, if there is none yet
    Create(OfflineArgs),
    /// Make the automatic families again from the games you play today
    ///
    /// Your own families stay as they are: only the ones gamepalate made and you never changed
    /// are replaced. To put back the families as they were, use Undo in the Profile of the app.
    Remake(OfflineArgs),
}

#[derive(Args)]
struct OfflineArgs {
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
                print_family(family, "Family", &files.catalog, &files.off_catalog);
            }
        }
        FamiliesCommand::Remake(remake) => {
            let files = signals::load(data_dir, remake.offline)?;
            let path = data_dir.join("families.toml");
            let date = jiff::Zoned::now().date().to_string();
            let outcome = remake_automatic_families(&path, &files.search_inputs(&[]), &date)?;

            let RemadeFamilies::Remade(remade) = &outcome else {
                println!("{}", not_remade_reason(&outcome));
                return Ok(());
            };
            println!(
                "Removed {} and made {}, in {}.",
                count(remade.removed.len()),
                count(remade.added.len()),
                path.display()
            );
            for family in &remade.removed {
                print_family(family, "Removed family", &files.catalog, &files.off_catalog);
            }
            for family in &remade.added {
                print_family(family, "New family", &files.catalog, &files.off_catalog);
            }
        }
    }
    Ok(())
}

fn print_family(
    family: &Family,
    label: &str,
    catalog: &Catalog,
    off_catalog: &BTreeMap<AppId, OffCatalogGame>,
) {
    println!();
    let kind = if family.automatic.is_some() {
        " (automatic)"
    } else {
        ""
    };
    println!("{label} \"{}\"{kind}:", family.name);
    for &game in &family.games {
        println!("  {}", game_label(game, catalog, off_catalog));
    }
}

fn count(families: usize) -> String {
    match families {
        1 => "1 automatic family".to_string(),
        _ => format!("{families} automatic families"),
    }
}

fn not_remade_reason(outcome: &RemadeFamilies) -> String {
    match outcome {
        RemadeFamilies::Remade(_) => String::new(),
        RemadeFamilies::NoNewFamilies => {
            "No new families: your families already cover the games you play".to_string()
        }
        RemadeFamilies::AlreadyMany => format!("You already have {MAX_FAMILIES} families"),
    }
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

    #[test]
    fn says_why_no_families_were_remade() {
        assert_eq!(
            not_remade_reason(&RemadeFamilies::NoNewFamilies),
            "No new families: your families already cover the games you play"
        );
        assert_eq!(
            not_remade_reason(&RemadeFamilies::AlreadyMany),
            "You already have 4 families"
        );
    }

    #[test]
    fn counts_the_families() {
        assert_eq!(count(0), "0 automatic families");
        assert_eq!(count(1), "1 automatic family");
        assert_eq!(count(4), "4 automatic families");
    }
}
