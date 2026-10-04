use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub champions: Vec<Champion>,
    pub augments: Vec<Augment>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Champion {
    pub id: u32,
    pub key: String,
    pub name_fr: String,
    pub name_en: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Augment {
    pub id: u32,
    pub name_id: String,
    pub name_fr: String,
    pub name_en: String,
    pub description_fr: String,
    pub description_en: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    pub name_fr: String,
    pub name_en: String,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Tier {
    S,
    A,
    B,
    C,
    D,
    F,
    #[default]
    Unknown,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::S => "S",
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::F => "F",
            Self::Unknown => "?",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AugmentStat {
    pub id: u32,
    pub tier: Tier,
    pub rank: Option<u32>,
    pub sample_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRoute {
    pub purchase_order: Vec<u32>,
    pub sample_count: Option<u64>,
    pub starters: Vec<u32>,
    pub boots: Vec<u32>,
    pub later_items: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemSynergy {
    pub item_id: u32,
    pub augment_id: u32,
    pub sample_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub champion_id: u32,
    pub source: String,
    pub patch: String,
    pub dataset: String,
    pub dataset_date: String,
    pub fetched_at: String,
    pub augments: Vec<AugmentStat>,
    pub stages: BTreeMap<u8, Vec<AugmentStat>>,
    pub builds: Vec<BuildRoute>,
    pub item_synergies: Vec<ItemSynergy>,
}
