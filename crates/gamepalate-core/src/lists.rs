use std::collections::BTreeMap;

/// A list of saved games. A new list gets the highest id plus one, so the order of the ids is
/// the order the lists were made in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ListId(pub u32);

/// The name of each list, oldest first.
pub type Lists = BTreeMap<ListId, String>;

pub const MAX_NAME_CHARS: usize = 40;

/// Why a name cannot be a list's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListNameError {
    Empty,
    TooLong,
    Taken,
}

/// The id a new list gets, or `None` when the highest id is already the last one.
pub fn next_id(lists: &Lists) -> Option<ListId> {
    match lists.keys().next_back() {
        Some(id) => id.0.checked_add(1).map(ListId),
        None => Some(ListId(1)),
    }
}

/// `name` without spaces around it, if a list can have it. `except` is the list being renamed,
/// which can keep its own name.
pub fn list_name(
    lists: &Lists,
    name: &str,
    except: Option<ListId>,
) -> Result<String, ListNameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ListNameError::Empty);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ListNameError::TooLong);
    }
    let lower = name.to_lowercase();
    if lists
        .iter()
        .any(|(&id, other)| Some(id) != except && other.to_lowercase() == lower)
    {
        return Err(ListNameError::Taken);
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lists() -> Lists {
        Lists::from([
            (ListId(2), "Co-op".to_string()),
            (ListId(5), "On sale".to_string()),
        ])
    }

    #[test]
    fn a_new_list_comes_after_the_highest_id() {
        assert_eq!(next_id(&Lists::new()), Some(ListId(1)));
        assert_eq!(next_id(&lists()), Some(ListId(6)));
    }

    #[test]
    fn there_is_no_id_after_the_last_one() {
        let lists = Lists::from([(ListId(u32::MAX), "Written by hand".to_string())]);

        assert_eq!(next_id(&lists), None);
    }

    #[test]
    fn a_name_loses_the_spaces_around_it() {
        assert_eq!(
            list_name(&lists(), "  Steam Deck ", None),
            Ok("Steam Deck".to_string())
        );
    }

    #[test]
    fn an_empty_name_is_refused() {
        for name in ["", "   "] {
            assert_eq!(list_name(&lists(), name, None), Err(ListNameError::Empty));
        }
    }

    #[test]
    fn a_name_has_at_most_40_characters() {
        let longest = "è".repeat(MAX_NAME_CHARS);
        assert_eq!(list_name(&lists(), &longest, None), Ok(longest.clone()));
        assert_eq!(
            list_name(&lists(), &format!("{longest}a"), None),
            Err(ListNameError::TooLong)
        );
    }

    #[test]
    fn a_name_in_use_is_refused_whatever_the_case() {
        assert_eq!(
            list_name(&lists(), "co-OP", None),
            Err(ListNameError::Taken)
        );
        assert_eq!(
            list_name(&lists(), "on sale", Some(ListId(2))),
            Err(ListNameError::Taken)
        );
    }

    #[test]
    fn a_list_can_keep_its_own_name() {
        assert_eq!(
            list_name(&lists(), "CO-OP", Some(ListId(2))),
            Ok("CO-OP".to_string())
        );
    }
}
