use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language: String,
    pub scan_interval_ms: u64,
    pub selected_augments: Vec<u32>,
    pub offer_stage: Option<u8>,
    pub show_builds: bool,
    pub synergy_rules: Vec<crate::domain::SynergyRule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "fr".into(),
            scan_interval_ms: 900,
            selected_augments: Vec::new(),
            offer_stage: None,
            show_builds: true,
            synergy_rules: Vec::new(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.language.as_str(), "fr" | "en") {
            bail!("language doit être fr ou en");
        }
        if !(400..=5000).contains(&self.scan_interval_ms) {
            bail!("scan_interval_ms doit être compris entre 400 et 5000");
        }
        if self.offer_stage.is_some_and(|s| !(1..=4).contains(&s)) {
            bail!("offer_stage doit être null ou compris entre 1 et 4");
        }
        if self.selected_augments.len() > 5 {
            bail!("Au plus cinq augmentations choisies");
        }
        if self.synergy_rules.len() > 100
            || self.synergy_rules.iter().any(|rule| {
                rule.id.is_empty()
                    || rule.id.len() > 64
                    || rule.offered == 0
                    || rule.requires.is_empty()
                    || rule.requires.len() > 5
                    || rule.requires.contains(&0)
                    || rule.explanation_fr.len() > 300
                    || rule.explanation_en.len() > 300
            })
        {
            bail!("Règle de synergie invalide ou trop volumineuse");
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read(path).context("Lecture des réglages")?;
        if data.len() > 64 * 1024 {
            bail!("Réglages trop volumineux");
        }
        let result: Self = serde_json::from_slice(&data).context("Réglages JSON invalides")?;
        result.validate()?;
        Ok(result)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(&temporary, path).context("Enregistrement des réglages")?;
        Ok(())
    }
}

pub fn app_directory() -> PathBuf {
    #[cfg(windows)]
    if let Some(directory) = crate::native::package_data_directory() {
        return directory;
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("MayhemLens")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_or_ambiguous_settings() {
        let mut config = Config::default();
        assert!(config.validate().is_ok());
        config.scan_interval_ms = 0;
        assert!(config.validate().is_err());
        config.scan_interval_ms = 900;
        config.offer_stage = Some(5);
        assert!(config.validate().is_err());
        config.offer_stage = None;
        config.language = "auto".into();
        assert!(config.validate().is_err());
    }
}
