use std::collections::{HashMap, HashSet, hash_map::Entry};

use crate::{ids::AppId, store_item::StoreItem, timestamp::Timestamp};

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    Month,
    Quarter,
    Year,
    Older,
}

impl Band {
    pub const ALL: [Band; 4] = [Band::Month, Band::Quarter, Band::Year, Band::Older];

    pub fn of(released: Option<Timestamp>, now: Timestamp) -> Band {
        let Some(released) = released else {
            return Band::Older;
        };
        match now.ms_since(released) {
            age if age < 0 => Band::Older,
            age if age < 30 * DAY_MS => Band::Month,
            age if age < 90 * DAY_MS => Band::Quarter,
            age if age < 365 * DAY_MS => Band::Year,
            _ => Band::Older,
        }
    }

    pub fn max_age_ms(self) -> i64 {
        match self {
            Band::Month => 20 * HOUR_MS,
            Band::Quarter => 7 * DAY_MS,
            Band::Year => 14 * DAY_MS,
            Band::Older => 30 * DAY_MS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Answer {
    at: Timestamp,
    released: Option<Timestamp>,
}

#[derive(Debug, Default)]
pub struct Answers {
    answers: HashMap<AppId, Answer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renewal {
    pub asked: Vec<AppId>,
    pub renewed: Vec<(Band, usize)>,
    pub still_expired: usize,
}

impl Answers {
    pub fn add(&mut self, at: Timestamp, asked: &[AppId], items: &[StoreItem]) {
        let released: HashMap<AppId, Option<Timestamp>> = items
            .iter()
            .map(|item| (item.id, item.release_date.filter(|_| item.sold())))
            .collect();

        for id in asked {
            if let Entry::Vacant(entry) = self.answers.entry(*id) {
                entry.insert(Answer {
                    at,
                    released: released.get(id).copied().flatten(),
                });
            }
        }
    }

    pub fn choose(
        &self,
        listed: &[AppId],
        changed: &[AppId],
        now: Timestamp,
        max_apps: usize,
    ) -> Renewal {
        let changed: HashSet<AppId> = changed.iter().copied().collect();
        let mut expired: Vec<(Band, Timestamp, usize, AppId)> = listed
            .iter()
            .enumerate()
            .filter(|(_, id)| !changed.contains(id))
            .filter_map(|(order, id)| {
                let answer = self.answers.get(id)?;
                let band = Band::of(answer.released, now);
                (now.ms_since(answer.at) > band.max_age_ms())
                    .then_some((band, answer.at, order, *id))
            })
            .collect();
        expired.sort_unstable();
        let still_expired = expired.len().saturating_sub(max_apps);
        expired.truncate(max_apps);

        let renewed = Band::ALL
            .iter()
            .map(|band| {
                let apps = expired.iter().filter(|(of, ..)| of == band).count();
                (*band, apps)
            })
            .collect();
        let chosen: HashSet<AppId> = expired.into_iter().map(|(.., id)| id).collect();
        Renewal {
            asked: listed
                .iter()
                .filter(|id| changed.contains(id) || chosen.contains(id))
                .copied()
                .collect(),
            renewed,
            still_expired,
        }
    }

    pub fn next_expiry(&self, listed: &[AppId], now: Timestamp) -> Option<Timestamp> {
        listed
            .iter()
            .filter_map(|id| self.answers.get(id))
            .map(|answer| Timestamp(answer.at.0 + Band::of(answer.released, now).max_age_ms()))
            .min()
    }
}

impl Renewal {
    pub fn renewed_apps(&self) -> usize {
        self.renewed.iter().map(|(_, apps)| apps).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{game::Reviews, store_item::ItemPrice};

    const NOW: Timestamp = Timestamp(1_793_638_800_000);

    fn ago(ms: i64) -> Timestamp {
        Timestamp(NOW.0 - ms)
    }

    fn item(id: u32, released: Timestamp) -> StoreItem {
        StoreItem {
            id: AppId(id),
            name: format!("Game {id}"),
            app_type: 0,
            is_free: false,
            is_early_access: false,
            is_coming_soon: false,
            release_date: Some(released),
            price: Some(ItemPrice {
                final_cents: 999,
                original_cents: 999,
                discount_percent: 0,
                discount_end: None,
            }),
            formatted_price: None,
            reviews: Reviews {
                total: 10,
                positive: 9,
            },
            tags: Vec::new(),
        }
    }

    fn ids(ids: &[u32]) -> Vec<AppId> {
        ids.iter().copied().map(AppId).collect()
    }

    fn answered(apps: &[(u32, Timestamp, StoreItem)]) -> Answers {
        let mut answers = Answers::default();
        for (id, at, item) in apps {
            answers.add(*at, &[AppId(*id)], std::slice::from_ref(item));
        }
        answers
    }

    fn renewed(renewal: &Renewal) -> Vec<usize> {
        renewal.renewed.iter().map(|(_, apps)| *apps).collect()
    }

    #[test]
    fn the_band_follows_the_release_date() {
        assert_eq!(Band::of(Some(ago(29 * DAY_MS)), NOW), Band::Month);
        assert_eq!(Band::of(Some(ago(30 * DAY_MS)), NOW), Band::Quarter);
        assert_eq!(Band::of(Some(ago(89 * DAY_MS)), NOW), Band::Quarter);
        assert_eq!(Band::of(Some(ago(90 * DAY_MS)), NOW), Band::Year);
        assert_eq!(Band::of(Some(ago(364 * DAY_MS)), NOW), Band::Year);
        assert_eq!(Band::of(Some(ago(365 * DAY_MS)), NOW), Band::Older);
        assert_eq!(Band::of(Some(Timestamp(NOW.0 + DAY_MS)), NOW), Band::Older);
        assert_eq!(Band::of(None, NOW), Band::Older);
    }

    #[test]
    fn each_band_expires_after_its_limit() {
        let cases = [
            (ago(10 * DAY_MS), 20 * HOUR_MS),
            (ago(60 * DAY_MS), 7 * DAY_MS),
            (ago(200 * DAY_MS), 14 * DAY_MS),
            (ago(1000 * DAY_MS), 30 * DAY_MS),
        ];
        for (released, limit) in cases {
            let answers = answered(&[
                (1, ago(limit + 1), item(1, released)),
                (2, ago(limit), item(2, released)),
            ]);

            let renewal = answers.choose(&ids(&[1, 2]), &[], NOW, 100);

            assert_eq!(renewal.asked, ids(&[1]), "limit {limit}");
        }
    }

    #[test]
    fn an_upcoming_game_that_came_out_is_renewed_like_a_new_one() {
        let released = ago(2 * DAY_MS);
        let answered_at = ago(3 * DAY_MS);
        let mut upcoming = item(1, released);
        upcoming.is_coming_soon = true;
        let answers = answered(&[(1, answered_at, upcoming)]);

        assert_eq!(Band::of(Some(released), answered_at), Band::Older);
        assert_eq!(answers.choose(&ids(&[1]), &[], NOW, 100).asked, ids(&[1]));
    }

    #[test]
    fn free_and_unsold_apps_are_in_the_last_band() {
        let released = ago(10 * DAY_MS);
        let mut free = item(1, released);
        free.is_free = true;
        let mut unpriced = item(2, released);
        unpriced.price = None;
        let mut dlc = item(3, released);
        dlc.app_type = 4;
        let at = ago(25 * DAY_MS);
        let mut answers = answered(&[(1, at, free), (2, at, unpriced), (3, at, dlc)]);
        answers.add(at, &ids(&[4]), &[]);

        let renewal = answers.choose(&ids(&[1, 2, 3, 4]), &[], NOW, 100);

        assert!(renewal.asked.is_empty());
        assert_eq!(
            answers
                .choose(&ids(&[1, 2, 3, 4]), &[], Timestamp(NOW.0 + 6 * DAY_MS), 100)
                .asked,
            ids(&[1, 2, 3, 4])
        );
    }

    #[test]
    fn the_first_answer_about_an_app_decides_its_age() {
        let mut answers = Answers::default();
        answers.add(ago(HOUR_MS), &ids(&[1]), &[item(1, ago(DAY_MS))]);
        answers.add(ago(10 * DAY_MS), &ids(&[1]), &[item(1, ago(DAY_MS))]);

        assert!(answers.choose(&ids(&[1]), &[], NOW, 100).asked.is_empty());
    }

    #[test]
    fn newer_bands_come_first_then_the_most_expired_then_the_list_order() {
        let answers = answered(&[
            (1, ago(40 * DAY_MS), item(1, ago(1000 * DAY_MS))),
            (2, ago(20 * DAY_MS), item(2, ago(200 * DAY_MS))),
            (3, ago(2 * DAY_MS), item(3, ago(10 * DAY_MS))),
            (4, ago(30 * DAY_MS), item(4, ago(200 * DAY_MS))),
            (5, ago(2 * DAY_MS), item(5, ago(10 * DAY_MS))),
        ]);
        let listed = ids(&[1, 2, 3, 4, 5]);
        let choose = |max_apps| {
            let mut asked = answers.choose(&listed, &[], NOW, max_apps).asked;
            asked.sort();
            asked
        };

        assert_eq!(choose(1), ids(&[3]));
        assert_eq!(choose(2), ids(&[3, 5]));
        assert_eq!(choose(3), ids(&[3, 4, 5]));
        assert_eq!(choose(4), ids(&[2, 3, 4, 5]));
        assert_eq!(choose(5), ids(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn the_cap_leaves_the_rest_expired_and_counts_each_band() {
        let answers = answered(&[
            (1, ago(2 * DAY_MS), item(1, ago(10 * DAY_MS))),
            (2, ago(2 * DAY_MS), item(2, ago(10 * DAY_MS))),
            (3, ago(20 * DAY_MS), item(3, ago(200 * DAY_MS))),
            (4, ago(40 * DAY_MS), item(4, ago(1000 * DAY_MS))),
        ]);

        let renewal = answers.choose(&ids(&[1, 2, 3, 4]), &[], NOW, 3);

        assert_eq!(renewal.asked, ids(&[1, 2, 3]));
        assert_eq!(renewed(&renewal), [2, 0, 1, 0]);
        assert_eq!(renewal.renewed_apps(), 3);
        assert_eq!(renewal.still_expired, 1);
    }

    #[test]
    fn changed_apps_are_asked_outside_the_cap_and_not_counted() {
        let answers = answered(&[
            (1, ago(2 * DAY_MS), item(1, ago(10 * DAY_MS))),
            (2, ago(HOUR_MS), item(2, ago(10 * DAY_MS))),
            (3, ago(2 * DAY_MS), item(3, ago(10 * DAY_MS))),
        ]);

        let renewal = answers.choose(&ids(&[1, 2, 3, 4]), &ids(&[1, 2, 4]), NOW, 1);

        assert_eq!(renewal.asked, ids(&[1, 2, 3, 4]));
        assert_eq!(renewal.renewed_apps(), 1);
        assert_eq!(renewal.still_expired, 0);
    }

    #[test]
    fn the_asked_apps_follow_the_list_order() {
        let answers = answered(&[
            (7, ago(2 * DAY_MS), item(7, ago(10 * DAY_MS))),
            (3, ago(40 * DAY_MS), item(3, ago(1000 * DAY_MS))),
        ]);

        let renewal = answers.choose(&ids(&[3, 5, 7]), &ids(&[5]), NOW, 100);

        assert_eq!(renewal.asked, ids(&[3, 5, 7]));
    }

    #[test]
    fn apps_no_longer_listed_are_not_renewed() {
        let answers = answered(&[(1, ago(40 * DAY_MS), item(1, ago(1000 * DAY_MS)))]);

        let renewal = answers.choose(&ids(&[2]), &[], NOW, 100);

        assert!(renewal.asked.is_empty());
        assert_eq!(renewal.renewed_apps(), 0);
    }

    #[test]
    fn the_next_expiry_is_the_earliest_among_the_listed_apps() {
        let answers = answered(&[
            (1, ago(DAY_MS), item(1, ago(1000 * DAY_MS))),
            (2, ago(HOUR_MS), item(2, ago(10 * DAY_MS))),
            (3, ago(10 * HOUR_MS), item(3, ago(10 * DAY_MS))),
        ]);

        assert_eq!(
            answers.next_expiry(&ids(&[1, 2, 3]), NOW),
            Some(Timestamp(NOW.0 + 10 * HOUR_MS))
        );
        assert_eq!(
            answers.next_expiry(&ids(&[1, 2]), NOW),
            Some(Timestamp(NOW.0 + 19 * HOUR_MS))
        );
        assert_eq!(
            answers.next_expiry(&ids(&[1]), NOW),
            Some(Timestamp(NOW.0 + 29 * DAY_MS))
        );
    }

    #[test]
    fn there_is_no_next_expiry_without_answers() {
        assert_eq!(Answers::default().next_expiry(&ids(&[1]), NOW), None);
    }
}
