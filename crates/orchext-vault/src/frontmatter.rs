use crate::{DocumentId, Visibility};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub id: DocumentId,

    #[serde(rename = "type")]
    pub type_: String,

    pub visibility: Visibility,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<NaiveDate>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<NaiveDate>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    #[serde(flatten)]
    pub extras: BTreeMap<String, serde_yml::Value>,
}
