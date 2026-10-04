#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{Context, Result, bail};
use mayhem_lens::{
    config::{Config, app_directory},
    data::DataStore,
    domain,
    model::Catalog,
};
use std::{env, path::PathBuf};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    #[cfg(windows)]
    if !args.is_empty() && args[0] != "run" {
        // Attach to an existing caller console only; never create a window.
        unsafe {
            let _ = windows::Win32::System::Console::AttachConsole(
                windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    if let Err(error) = execute(args) {
        eprintln!("Mayhem Lens : {error:#}");
        let _ = std::fs::create_dir_all(app_directory());
        let _ = std::fs::write(app_directory().join("last-error.txt"), format!("{error:#}"));
        std::process::exit(1);
    }
}

fn execute(mut args: Vec<String>) -> Result<()> {
    let mut cache = app_directory().join("cache");
    let mut config_path = app_directory().join("config.json");
    let mut index = 0;
    while index < args.len() {
        if matches!(args[index].as_str(), "--cache" | "--config") {
            if index + 1 >= args.len() {
                bail!("{} attend un chemin", args[index]);
            }
            let path = PathBuf::from(args.remove(index + 1));
            if args[index] == "--cache" {
                cache = path;
            } else {
                config_path = path;
            }
            args.remove(index);
        } else {
            index += 1;
        }
    }
    let command = args.first().map(String::as_str).unwrap_or("run");
    match command {
        "--help" | "-h" | "help" => {
            println!(
                "Mayhem Lens {} — ARAM Mayhem uniquement\n\
                run                               Démarrer l'overlay (MSIX / Windows OCR)\n\
                sync <champion ID ou nom>          Charger les données Mayhem du champion\n\
                recommend <champion> <nom|ID>...    Lire les tiers du cache, sans réseau\n\
                config                            Afficher les réglages et leur chemin\n\
                config language fr|en             Choisir la langue\n\
                config stage 1|2|3|4|unknown       Indiquer explicitement le choix\n\
                config selected <ID>...           Corriger les augmentations précédentes\n\
                config reset                      Effacer choix et stade\n\
                status                            Lire l'état du worker\n\
                diagnose                          Capacités Windows et API locale en lecture\n\
                update check                      Vérifier une mise à jour sans l'installer\n\
                update                            Préparer la mise à jour pour le prochain lancement\n\
                --cache <dossier>                  Choisir un cache séparé\n\
                --config <fichier>                 Choisir des réglages séparés\n\
                Ctrl+Shift+M : relire les cartes ; Ctrl+Shift+1/2/3 : confirmer un choix ;\n\
                Ctrl+Shift+Q : quitter. Aucun raccourci ne clique dans LoL.",
                env!("CARGO_PKG_VERSION")
            );
        }
        "--version" | "-V" => println!("Mayhem Lens {}", env!("CARGO_PKG_VERSION")),
        "sync" => {
            if args.len() != 2 {
                bail!("Usage : sync <champion ID ou nom>");
            }
            let store = DataStore::new(cache)?;
            let id = match args[1].parse::<u32>() {
                Ok(id) => id,
                Err(_) => champion_id(&store.catalog()?, &args[1])?,
            };
            let snapshot = store.sync_champion(id)?;
            let catalog = store.catalog()?;
            println!(
                "{} — champion {} — patch {} — dataset {} — {} augmentations Mayhem, {} routes",
                snapshot.source,
                id,
                snapshot.patch,
                snapshot.dataset_date,
                snapshot.augments.len(),
                snapshot.builds.len()
            );
            println!(
                "Catalogue : {} noms FR/EN. Dataset : {}",
                catalog.augments.len(),
                snapshot.dataset
            );
        }
        "recommend" => {
            if args.len() < 3 {
                bail!("Usage : recommend <champion> <ID ou nom d'augmentation>...");
            }
            let store = DataStore::new(cache)?;
            let catalog = store
                .cached_catalog()
                .context("Catalogue absent du cache : lancer sync une première fois")?;
            let id = champion_id(&catalog, &args[1])?;
            let snapshot = store.cached_champion(id)?;
            let settings = Config::load(&config_path)?;
            let mut offers = Vec::new();
            for text in &args[2..] {
                let id = text
                    .parse()
                    .ok()
                    .or_else(|| domain::match_augment(text, &catalog).map(|m| m.id));
                offers.push(id.with_context(|| {
                    format!("Augmentation Mayhem ambiguë ou inconnue : {text}")
                })?);
            }
            let mut recommendations = domain::recommend_localized(
                &snapshot,
                &catalog,
                &offers,
                &settings.selected_augments,
                settings.offer_stage,
                &settings.language,
            );
            domain::apply_rules(
                &mut recommendations,
                &settings.selected_augments,
                &settings.synergy_rules,
                &settings.language,
            );
            for result in recommendations {
                println!(
                    "{} — {} : {}",
                    result.tier,
                    augment_name(&catalog, result.augment_id, &settings.language),
                    result.explanation
                );
                for rule in result.synergies {
                    println!("  {rule}");
                }
            }
            for build in domain::build_advice_localized(
                &snapshot,
                &catalog,
                &settings.selected_augments,
                &settings.language,
            )
            .into_iter()
            .take(2)
            {
                println!(
                    "{} : {}. {}",
                    build.label,
                    build
                        .item_ids
                        .iter()
                        .map(|id| item_name(&catalog, *id, &settings.language))
                        .collect::<Vec<_>>()
                        .join(" → "),
                    build.reason
                );
            }
        }
        "config" => configure(&args[1..], &config_path)?,
        "update" => {
            if args.len() > 2 || (args.len() == 2 && args[1] != "check") {
                bail!("Usage : update [check]");
            }
            #[cfg(windows)]
            {
                let status = if args.len() == 2 {
                    mayhem_lens::update::check()?
                } else {
                    mayhem_lens::update::check_and_stage()?
                };
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
            #[cfg(not(windows))]
            bail!("Les mises à jour natives nécessitent Windows.");
        }
        "status" => {
            let path = app_directory().join("runtime-status.json");
            if path.exists() {
                println!("{}", std::fs::read_to_string(path)?);
            } else {
                println!("Aucun état enregistré. L'overlay n'a pas encore été lancé.");
            }
        }
        "diagnose" => {
            #[cfg(windows)]
            println!("{}", mayhem_lens::native::diagnostics()?);
            match mayhem_lens::game::read_game()? {
                Some(game) => println!(
                    "Partie Mayhem : {}, niveau {}, temps {:.0}s",
                    game.champion_key, game.level, game.game_time
                ),
                None => println!("API locale : aucune partie ARAM Mayhem détectée."),
            }
            println!("Réglages : {}", config_path.display());
        }
        "run" => {
            if args.len() > 1 {
                bail!("Usage : run [--cache <dossier>]");
            }
            #[cfg(windows)]
            mayhem_lens::app::run(cache, config_path)?;
            #[cfg(not(windows))]
            bail!("L'overlay nécessite Windows.");
        }
        _ => bail!("Commande inconnue. Utiliser --help."),
    }
    Ok(())
}

fn configure(args: &[String], path: &std::path::Path) -> Result<()> {
    let mut config = Config::load(path)?;
    if let Some(command) = args.first().map(String::as_str) {
        match command {
            "language" if args.len() == 2 => config.language.clone_from(&args[1]),
            "stage" if args.len() == 2 => {
                config.offer_stage = if args[1] == "unknown" {
                    None
                } else {
                    Some(args[1].parse()?)
                }
            }
            "selected" => {
                config.selected_augments = args[1..]
                    .iter()
                    .map(|s| s.parse())
                    .collect::<std::result::Result<_, _>>()?
            }
            "reset" if args.len() == 1 => {
                config.selected_augments.clear();
                config.offer_stage = None;
            }
            _ => bail!(
                "Usage : config [language fr|en | stage 1..4|unknown | selected <ID>... | reset]"
            ),
        }
        config.save(path)?;
    }
    println!(
        "{}\n{}",
        path.display(),
        serde_json::to_string_pretty(&config)?
    );
    Ok(())
}

fn champion_id(catalog: &Catalog, input: &str) -> Result<u32> {
    if let Ok(id) = input.parse() {
        return Ok(id);
    }
    let input = input.to_lowercase().replace([' ', '-', '\''], "");
    catalog
        .champions
        .iter()
        .find(|c| {
            [&c.key, &c.name_fr, &c.name_en]
                .into_iter()
                .any(|name| name.to_lowercase().replace([' ', '-', '\''], "") == input)
        })
        .map(|c| c.id)
        .context("Champion inconnu")
}

fn augment_name(catalog: &Catalog, id: u32, language: &str) -> String {
    catalog
        .augments
        .iter()
        .find(|a| a.id == id)
        .map(|a| {
            if language == "en" {
                a.name_en.clone()
            } else {
                a.name_fr.clone()
            }
        })
        .unwrap_or_else(|| format!("#{id}"))
}

fn item_name(catalog: &Catalog, id: u32, language: &str) -> String {
    catalog
        .items
        .iter()
        .find(|a| a.id == id)
        .map(|a| {
            if language == "en" {
                a.name_en.clone()
            } else {
                a.name_fr.clone()
            }
        })
        .unwrap_or_else(|| format!("#{id}"))
}
