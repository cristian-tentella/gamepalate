use std::collections::{BTreeMap, HashMap};

use gamepalate_core::{catalog::TagName, ids::TagId};
use serde::Deserialize;
use serde_json::Value;

use crate::transport::{Get, SteamError, parse};

const ENDPOINT: &str = "IStoreService/GetTagList/v1";

#[derive(Deserialize)]
struct TagListEnvelope {
    response: TagListResponse,
}

#[derive(Deserialize)]
struct TagListResponse {
    #[serde(default)]
    tags: Vec<SteamTag>,
}

#[derive(Debug, PartialEq, Deserialize)]
struct SteamTag {
    tagid: u32,
    name: String,
}

pub async fn get_tags<S: Get>(transport: &S) -> Result<BTreeMap<TagId, TagName>, SteamError> {
    let english = get_tag_list(transport, "english").await?;
    let italian = get_tag_list(transport, "italian").await?;
    Ok(merge_languages(english, italian))
}

async fn get_tag_list<S: Get>(transport: &S, language: &str) -> Result<Vec<SteamTag>, SteamError> {
    let value = transport
        .get(ENDPOINT, &[("language", language)], false)
        .await?;
    parse_tag_list(value)
}

fn merge_languages(english: Vec<SteamTag>, italian: Vec<SteamTag>) -> BTreeMap<TagId, TagName> {
    let mut italian: HashMap<u32, String> = italian
        .into_iter()
        .map(|tag| (tag.tagid, tag.name))
        .collect();

    let mut tags: BTreeMap<TagId, TagName> = english
        .into_iter()
        .map(|tag| {
            let italian_name = italian
                .remove(&tag.tagid)
                .unwrap_or_else(|| tag.name.clone());
            (
                TagId(tag.tagid),
                TagName {
                    english: tag.name,
                    italian: italian_name,
                },
            )
        })
        .collect();

    tags.extend(italian.into_iter().map(|(tagid, name)| {
        (
            TagId(tagid),
            TagName {
                english: name.clone(),
                italian: name,
            },
        )
    }));

    tags
}

fn parse_tag_list(value: Value) -> Result<Vec<SteamTag>, SteamError> {
    Ok(parse::<TagListEnvelope>(ENDPOINT, value)?.response.tags)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tag(tagid: u32, name: &str) -> SteamTag {
        SteamTag {
            tagid,
            name: name.to_string(),
        }
    }

    fn names(tags: &BTreeMap<TagId, TagName>, tagid: u32) -> (&str, &str) {
        let tag = &tags[&TagId(tagid)];
        (&tag.english, &tag.italian)
    }

    #[test]
    fn merges_the_two_languages_by_tagid() {
        let english = vec![tag(19, "Action"), tag(21, "Adventure")];
        let italian = vec![tag(21, "Avventura"), tag(19, "Azione")];

        let tags = merge_languages(english, italian);

        assert_eq!(tags.len(), 2);
        assert_eq!(names(&tags, 19), ("Action", "Azione"));
        assert_eq!(names(&tags, 21), ("Adventure", "Avventura"));
    }

    #[test]
    fn a_name_missing_in_one_language_takes_the_other() {
        let english = vec![tag(19, "Action")];
        let italian = vec![tag(492, "Indie")];

        let tags = merge_languages(english, italian);

        assert_eq!(names(&tags, 19), ("Action", "Action"));
        assert_eq!(names(&tags, 492), ("Indie", "Indie"));
    }

    #[test]
    fn reads_the_tags_and_ignores_unknown_fields() {
        let value = json!({
            "response": {
                "version_hash": "abc",
                "tags": [{ "tagid": 19, "name": "Action", "extra": true }]
            }
        });

        let tags = parse_tag_list(value).unwrap();

        assert_eq!(
            tags,
            vec![SteamTag {
                tagid: 19,
                name: "Action".to_string()
            }]
        );
    }

    #[test]
    fn missing_tags_are_empty() {
        let tags = parse_tag_list(json!({ "response": {} })).unwrap();

        assert!(tags.is_empty());
    }

    #[test]
    fn a_wrong_type_is_a_shape_error_naming_the_endpoint() {
        let value = json!({ "response": { "tags": [{ "tagid": "19", "name": "Action" }] } });

        let error = parse_tag_list(value).unwrap_err();

        assert!(matches!(error, SteamError::Shape { .. }));
        assert!(error.to_string().starts_with(ENDPOINT));
    }
}
