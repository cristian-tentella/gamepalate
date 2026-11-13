use gamepalate_core::{search::SearchInputs, taste::game_name};
use gamepalate_store::families_edit::restore_families;

use super::{FAMILIES_FILE, Gamepalate};
use crate::{
    families::{
        CreatedFamilies, FamiliesChanged, FamiliesRemade, FamiliesUndo, FamilyChange,
        FamilyChangeError, FamilyGame, RemadeFamilies, RemakePreview, automatic_family_names,
        changed_families, family_games, new_played_games, open_error, remake_automatic_families,
        remake_preview,
    },
    log::failed,
};

impl Gamepalate {
    pub fn change_families(
        &self,
        change: &FamilyChange,
    ) -> Result<FamiliesChanged, FamilyChangeError> {
        let mut families = self.lock_families();
        let changed = changed_families(
            &self.data_dir.join(FAMILIES_FILE),
            &families,
            change,
            |id| {
                let name = game_name(id, &self.files.catalog, &self.files.off_catalog);
                (name != format!("#{}", id.0)).then_some(name)
            },
        )
        .inspect_err(failed("could not save the families"))?;
        *families = changed.families;

        Ok(FamiliesChanged {
            undo: FamiliesUndo {
                snapshot: changed.snapshot,
                discover_family: self.follow_renamed(&changed.renamed),
                families_played: None,
            },
            gone: changed
                .renamed
                .into_iter()
                .filter(|(_, to)| to.is_none())
                .map(|(from, _)| from)
                .collect(),
        })
    }

    pub fn remake_preview(&self) -> RemakePreview {
        self.with_inputs(remake_preview)
    }

    // Only the automatic families change; the undo puts back the file as it was.
    pub fn remake_families(&self) -> Result<FamiliesRemade, FamilyChangeError> {
        let votes = self.lock_votes();
        let corrections = self.lock_corrections();
        let mut families = self.lock_families();
        let outcome = remake_automatic_families(
            &self.data_dir.join(FAMILIES_FILE),
            &SearchInputs {
                votes: &votes,
                corrections: &corrections,
                families: &families,
                ..self.files.search_inputs(&self.vectors)
            },
            &jiff::Zoned::now().date().to_string(),
        )
        .map_err(open_error)
        .inspect_err(failed("could not remake the families"))?;

        let RemadeFamilies::Remade(remade) = outcome else {
            return Ok(FamiliesRemade {
                outcome,
                undo: None,
            });
        };
        *families = remade.families.clone();
        let undo = FamiliesUndo {
            snapshot: remade.snapshot.clone(),
            discover_family: self.follow_renamed(&remade.renamed()),
            families_played: self.remember_played(&remade.played),
        };
        Ok(FamiliesRemade {
            outcome: RemadeFamilies::Remade(remade),
            undo: Some(undo),
        })
    }

    pub fn undo_families(&self, undo: &FamiliesUndo) -> Result<(), FamilyChangeError> {
        let mut families = self.lock_families();
        *families = restore_families(&self.data_dir.join(FAMILIES_FILE), &undo.snapshot)
            .map_err(FamilyChangeError::Save)
            .inspect_err(failed("could not undo the families"))?;
        if let Some(family) = &undo.discover_family {
            let _ = self.change_gui_settings(|settings| settings.search.family = family.clone());
        }
        if let Some(played) = &undo.families_played {
            let _ = self.change_gui_settings(|settings| settings.families_played = played.clone());
        }
        Ok(())
    }

    // The families made from the played games when this session opened the data, if any.
    pub fn created_families(&self) -> Option<&CreatedFamilies> {
        self.created_families.as_ref()
    }

    // How many played games are new since the families were made, once there are enough.
    pub fn new_played_games(&self) -> Option<usize> {
        let seen = self.gui_settings().families_played?;
        self.with_inputs(|inputs| new_played_games(inputs, Some(&seen)))
    }

    pub fn family_games(&self, family: &str) -> Vec<FamilyGame> {
        family_games(&self.lock_families(), family, |id| {
            game_name(id, &self.files.catalog, &self.files.off_catalog)
        })
    }

    pub fn family_names(&self) -> Vec<String> {
        self.lock_families()
            .families
            .iter()
            .map(|family| family.name.clone())
            .collect()
    }

    pub fn automatic_family_names(&self) -> Vec<String> {
        automatic_family_names(&self.lock_families())
    }
}
