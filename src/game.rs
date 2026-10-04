//! Read-only loopback Live Client Data API. No account credentials or LCU access.
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde_json::Value;
use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct GameState {
    pub champion_key: String,
    pub champion_name: String,
    pub level: u32,
    pub game_time: f64,
}

const LIVE_URL: &str = "https://127.0.0.1:2999/liveclientdata/allgamedata";
static CLIENT: OnceLock<Client> = OnceLock::new();

pub fn read_game() -> Result<Option<GameState>> {
    let client = match CLIENT.get() {
        Some(client) => client,
        None => {
            let client = Client::builder()
                // Riot's local endpoint uses a self-signed certificate. This client is
                // restricted to the literal loopback URL and cannot follow redirects.
                .danger_accept_invalid_certs(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_millis(800))
                .connect_timeout(Duration::from_millis(300))
                .build()?;
            let _ = CLIENT.set(client);
            CLIENT.get().context("Client local indisponible")?
        }
    };
    let response = match client.get(LIVE_URL).send() {
        Ok(response) => response,
        Err(error) if error.is_connect() || error.is_timeout() => return Ok(None),
        Err(_) => anyhow::bail!("L'API locale du jeu est indisponible"),
    };
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let mut response = response
        .error_for_status()
        .context("Réponse de l'API locale du jeu")?;
    const LIMIT: u64 = 2 * 1024 * 1024;
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= LIMIT),
        "Réponse de l'API locale trop volumineuse"
    );
    let mut bytes = Vec::new();
    response.by_ref().take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "Réponse de l'API locale trop volumineuse"
    );
    let raw: Value = serde_json::from_slice(&bytes).context("État local du jeu invalide")?;
    parse_game(&raw)
}

fn parse_game(raw: &Value) -> Result<Option<GameState>> {
    let Some(game) = raw.get("gameData").and_then(Value::as_object) else {
        return Ok(None);
    };
    // KIWI was observed directly in Mayhem. ARAM/CHERRY are not substitutes.
    if game.get("gameMode").and_then(Value::as_str) != Some("KIWI") {
        return Ok(None);
    }
    let Some(active) = raw.get("activePlayer") else {
        return Ok(None);
    };
    let Some(players) = raw.get("allPlayers").and_then(Value::as_array) else {
        return Ok(None);
    };
    let active_riot_id = nonempty_text(active, "riotId");
    let active_summoner = nonempty_text(active, "summonerName");
    let matches: Vec<_> = players
        .iter()
        .filter(|player| {
            if let Some(riot_id) = active_riot_id {
                nonempty_text(player, "riotId") == Some(riot_id)
                    || nonempty_text(player, "summonerName") == Some(riot_id)
            } else if let Some(summoner) = active_summoner {
                nonempty_text(player, "summonerName") == Some(summoner)
            } else {
                false
            }
        })
        .collect();
    ensure!(matches.len() <= 1, "Identité locale ambiguë");
    let Some(player) = matches.first() else {
        return Ok(None);
    };
    let champion_key = nonempty_text(player, "rawChampionName")
        .and_then(|key| key.strip_prefix("game_character_displayname_"))
        .filter(|key| !key.is_empty())
        .context("Identifiant du champion local absent")?;
    let champion_name =
        nonempty_text(player, "championName").context("Nom du champion local absent")?;
    let level = active
        .get("level")
        .and_then(Value::as_u64)
        .context("Niveau du joueur local absent")?;
    ensure!((1..=30).contains(&level), "Niveau du joueur local invalide");
    let game_time = game
        .get("gameTime")
        .and_then(Value::as_f64)
        .context("Temps de partie absent")?;
    ensure!(
        game_time.is_finite() && game_time >= 0.0,
        "Temps de partie invalide"
    );
    Ok(Some(GameState {
        champion_key: champion_key.to_owned(),
        champion_name: champion_name.to_owned(),
        level: level as u32,
        game_time,
    }))
}

fn nonempty_text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> Value {
        json!({"gameData":{"gameMode":"KIWI","gameTime":12.5},"activePlayer":{"riotId":"Local#TEST","level":4},"allPlayers":[{"riotId":"Other#TEST","rawChampionName":"game_character_displayname_Other","championName":"Other"},{"riotId":"Local#TEST","rawChampionName":"game_character_displayname_Example","championName":"Exemple"}]})
    }

    #[test]
    fn identifies_the_local_player_instead_of_the_first_player() {
        let state = parse_game(&payload()).unwrap().unwrap();
        assert_eq!(state.champion_key, "Example");
        assert_eq!(state.level, 4);
        assert_eq!(state.game_time, 12.5);
    }

    #[test]
    fn rejects_classic_aram_arena_and_unknown_modes() {
        for mode in ["ARAM", "CHERRY", "CLASSIC", "KIWI_JADE", "UNKNOWN"] {
            let mut raw = payload();
            raw["gameData"]["gameMode"] = json!(mode);
            assert!(parse_game(&raw).unwrap().is_none());
        }
    }

    #[test]
    fn never_guesses_an_unmatched_local_player() {
        let mut raw = payload();
        raw["activePlayer"]["riotId"] = json!("Missing#TEST");
        assert!(parse_game(&raw).unwrap().is_none());
    }

    #[test]
    fn rejects_ambiguous_or_invalid_game_state() {
        let mut raw = payload();
        let duplicate = raw["allPlayers"][1].clone();
        raw["allPlayers"].as_array_mut().unwrap().push(duplicate);
        assert!(parse_game(&raw).is_err());
        let mut raw = payload();
        raw["gameData"]["gameTime"] = json!(-1);
        assert!(parse_game(&raw).is_err());
    }
}
