use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShortcutConfig {
    pub scan: String,
    pub quit: String,
    pub slot_1: String,
    pub slot_2: String,
    pub slot_3: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self {
            scan: "Ctrl+Shift+M".into(),
            quit: "Ctrl+Shift+Q".into(),
            slot_1: "Ctrl+Shift+1".into(),
            slot_2: "Ctrl+Shift+2".into(),
            slot_3: "Ctrl+Shift+3".into(),
        }
    }
}

impl ShortcutConfig {
    pub fn values(&self) -> [&str; 5] {
        [
            &self.scan,
            &self.quit,
            &self.slot_1,
            &self.slot_2,
            &self.slot_3,
        ]
    }

    pub fn validate(&self) -> Result<()> {
        let mut bindings = Vec::new();
        for text in self.values() {
            let binding = parse_shortcut(text)?;
            if bindings.contains(&binding) {
                bail!("Deux actions utilisent le même raccourci : {text}");
            }
            bindings.push(binding);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedShortcut {
    /// Win32 modifier bits (Alt=1, Ctrl=2, Shift=4, Win=8), without NOREPEAT.
    pub modifiers: u32,
    pub virtual_key: u32,
}

/// Parsing is pure: a valid binding may still conflict with another application.
pub fn parse_shortcut(value: &str) -> Result<ParsedShortcut> {
    if value.len() > 64 {
        bail!("Raccourci trop long");
    }
    let parts: Vec<_> = value.split('+').map(str::trim).collect();
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
        bail!("Raccourci invalide : utiliser par exemple Ctrl+Shift+M");
    }
    let mut modifiers = 0;
    for part in &parts[..parts.len() - 1] {
        let bit = match part.to_ascii_lowercase().as_str() {
            "alt" => 1,
            "ctrl" | "control" => 2,
            "shift" => 4,
            "win" | "windows" => 8,
            _ => bail!("Modificateur de raccourci invalide : {part}"),
        };
        if modifiers & bit != 0 {
            bail!("Modificateur répété dans le raccourci : {part}");
        }
        modifiers |= bit;
    }
    if modifiers & (1 | 2 | 8) == 0 {
        bail!("Un raccourci doit inclure Ctrl, Alt ou Win");
    }
    let key = parts[parts.len() - 1].to_ascii_uppercase();
    let virtual_key = if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() {
        u32::from(key.as_bytes()[0])
    } else if let Some(number) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if !(1..=24).contains(&number) {
            bail!("Touche de raccourci invalide : {key}");
        }
        0x70 + number - 1
    } else {
        bail!("Touche de raccourci invalide : utiliser A–Z, 0–9 ou F1–F24");
    };
    Ok(ParsedShortcut {
        modifiers,
        virtual_key,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language: String,
    pub scan_interval_ms: u64,
    pub selected_augments: Vec<u32>,
    pub offer_stage: Option<u8>,
    pub show_builds: bool,
    pub synergy_rules: Vec<crate::domain::SynergyRule>,
    pub auto_stage: bool,
    pub minimum_match_confidence: f32,
    pub overlay_scale: f32,
    pub overlay_opacity: u8,
    pub overlay_offset_x: i32,
    pub overlay_offset_y: i32,
    pub shortcuts: ShortcutConfig,
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
            auto_stage: true,
            minimum_match_confidence: 0.95,
            overlay_scale: 1.0,
            overlay_opacity: 94,
            overlay_offset_x: 0,
            overlay_offset_y: 0,
            shortcuts: ShortcutConfig::default(),
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
        if !self.minimum_match_confidence.is_finite()
            || !(0.90..=1.0).contains(&self.minimum_match_confidence)
        {
            bail!("La similarité minimale du nom doit être comprise entre 0,90 et 1,00");
        }
        if !self.overlay_scale.is_finite() || !(0.75..=1.5).contains(&self.overlay_scale) {
            bail!("La taille de l'overlay doit être comprise entre 0,75 et 1,50");
        }
        if !(30..=100).contains(&self.overlay_opacity) {
            bail!("L'opacité de l'overlay doit être comprise entre 30 et 100 %");
        }
        if !(-1000..=1000).contains(&self.overlay_offset_x)
            || !(-1000..=1000).contains(&self.overlay_offset_y)
        {
            bail!("Les décalages de l'overlay doivent être compris entre -1000 et 1000 pixels");
        }
        self.shortcuts.validate()?;
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
        let _lock = CONFIG_WRITE_LOCK
            .lock()
            .map_err(|_| anyhow::anyhow!("Verrou des réglages indisponible"))?;
        self.save_unlocked(path)
    }

    fn save_unlocked(&self, path: &Path) -> Result<()> {
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

/// Serialize read-modify-write operations inside this process. The closure sees
/// the latest file, so editing preferences cannot lose a concurrently saved pick.
pub fn modify(path: &Path, edit: impl FnOnce(&mut Config) -> Result<()>) -> Result<Config> {
    let _lock = CONFIG_WRITE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Verrou des réglages indisponible"))?;
    let mut config = Config::load(path)?;
    edit(&mut config)?;
    config.save_unlocked(path)?;
    Ok(config)
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

    #[test]
    fn legacy_json_loads_new_preferences_without_changing_choices() {
        let config: Config = serde_json::from_str(r#"{"language":"en","scan_interval_ms":1200,"selected_augments":[22],"offer_stage":2,"show_builds":false}"#).unwrap();
        config.validate().unwrap();
        assert!(config.auto_stage);
        assert_eq!(config.offer_stage, Some(2)); // Manual override still wins.
        assert_eq!(config.minimum_match_confidence, 0.95);
        assert_eq!(config.overlay_scale, 1.0);
        assert_eq!(config.overlay_opacity, 94);
        assert_eq!(config.overlay_offset_x, 0);
        assert_eq!(config.shortcuts, ShortcutConfig::default());
        assert_eq!(config.selected_augments, [22]);
    }

    #[test]
    fn rejects_nonfinite_preferences_and_equivalent_duplicate_shortcuts() {
        for confidence in [0.89, 1.01, f32::NAN, f32::INFINITY] {
            assert!(
                Config {
                    minimum_match_confidence: confidence,
                    ..Config::default()
                }
                .validate()
                .is_err()
            );
        }
        for scale in [0.74, 1.51, f32::NAN, f32::NEG_INFINITY] {
            assert!(
                Config {
                    overlay_scale: scale,
                    ..Config::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Config {
                overlay_offset_x: 1001,
                ..Config::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                overlay_opacity: 29,
                ..Config::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                overlay_opacity: 101,
                ..Config::default()
            }
            .validate()
            .is_err()
        );
        let keys = ShortcutConfig {
            quit: "shift + control + m".into(),
            ..ShortcutConfig::default()
        };
        assert!(keys.validate().is_err());
        assert_eq!(
            parse_shortcut("Alt+F8").unwrap(),
            ParsedShortcut {
                modifiers: 1,
                virtual_key: 0x77
            }
        );
        for key in [
            "M",
            "Shift+M",
            "Ctrl+Ctrl+M",
            "Ctrl+",
            "Ctrl+F25",
            "Ctrl+é",
            "Ctrl+Space",
        ] {
            assert!(parse_shortcut(key).is_err(), "{key}");
        }
    }

    #[test]
    fn concurrent_modifications_preserve_each_new_pick_and_reject_invalid_write() {
        use std::{
            sync::{Arc, Barrier},
            time::{SystemTime, UNIX_EPOCH},
        };
        let directory = std::env::temp_dir().join(format!(
            "MayhemLens-config-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("config.json");
        Config::default().save(&path).unwrap();
        let barrier = Arc::new(Barrier::new(5));
        let workers: Vec<_> = (1..=5)
            .map(|id| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    modify(&path, |config| {
                        config.selected_augments.push(id);
                        Ok(())
                    })
                    .unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let mut config = Config::load(&path).unwrap();
        config.selected_augments.sort_unstable();
        assert_eq!(config.selected_augments, [1, 2, 3, 4, 5]);
        assert!(
            modify(&path, |config| {
                config.overlay_opacity = 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(Config::load(&path).unwrap().overlay_opacity, 94);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
