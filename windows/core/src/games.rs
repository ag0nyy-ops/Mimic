//! The game list: parsing discord's "detectable applications" json (and our
//! slimmed down copy of it in `shared/games.json`, which has the same shape)
//! and searching it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One entry of `games.json`. The id is a discord application id and must
/// always come from discord's list, never typed in by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Error)]
pub enum GamesError {
    #[error("the game list isn't valid json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("only {0} usable games in that data, which doesn't look right")]
    TooFew(usize),
}

/// Fewer than this and we assume the response is wrong (a rate limit page, an
/// api change) and keep what we had.
pub const MIN_EXPECTED_GAMES: usize = 100;

fn is_snowflake(id: &str) -> bool {
    (15..=25).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// Accepts discord's raw response or our slim file. Keeps `id` + `name`, drops
/// entries without both, drops duplicate ids, sorts by name.
pub fn parse_games(raw: &str) -> Result<Vec<Game>, GamesError> {
    #[derive(Deserialize)]
    struct Entry {
        id: Option<String>,
        name: Option<String>,
    }
    let entries: Vec<Entry> = serde_json::from_str(raw)?;

    let mut seen = std::collections::HashSet::new();
    let mut games = Vec::with_capacity(entries.len());
    for e in entries {
        let (Some(id), Some(name)) = (e.id, e.name) else {
            continue;
        };
        let name = name.trim().to_owned();
        if name.is_empty() || !is_snowflake(&id) || !seen.insert(id.clone()) {
            continue;
        }
        games.push(Game { id, name });
    }
    if games.len() < MIN_EXPECTED_GAMES {
        return Err(GamesError::TooFew(games.len()));
    }
    games.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(games)
}

/// Lowercase and strip the common latin accents, so "pokemon" finds "Pokémon".
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct GameList {
    games: Vec<Game>,
    folded: Vec<String>,
}

impl GameList {
    pub fn new(games: Vec<Game>) -> Self {
        let folded = games.iter().map(|g| fold(&g.name)).collect();
        Self { games, folded }
    }

    pub fn from_json(raw: &str) -> Result<Self, GamesError> {
        Ok(Self::new(parse_games(raw)?))
    }

    pub fn len(&self) -> usize {
        self.games.len()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Game> {
        self.games.iter().find(|g| g.id == id)
    }

    /// Case and accent insensitive. Every word you type has to appear in the
    /// name. Best matches first: exact, starts with it, a word starts with it,
    /// then anywhere; shorter names win ties. An empty query returns the first
    /// `limit` games alphabetically.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Game> {
        let q = fold(query.trim());
        if q.is_empty() {
            return self.games.iter().take(limit).cloned().collect();
        }
        let tokens: Vec<&str> = q.split_whitespace().collect();

        let mut hits: Vec<(u8, usize, usize)> = Vec::new(); // (rank, name length, index)
        for (i, name) in self.folded.iter().enumerate() {
            if !tokens.iter().all(|t| name.contains(t)) {
                continue;
            }
            let first = tokens[0];
            let rank = if *name == q {
                0
            } else if name.starts_with(&q) {
                1
            } else if name.starts_with(first) || name.contains(&format!(" {first}")) {
                2
            } else {
                3
            };
            hits.push((rank, name.chars().count(), i));
        }
        hits.sort_unstable();
        hits.into_iter()
            .take(limit)
            .map(|(_, _, i)| self.games[i].clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: u64, name: &str) -> Game {
        Game {
            id: id.to_string(),
            name: name.into(),
        }
    }

    /// Enough padding entries to pass the sanity minimum.
    fn padded(extra: &[(&str, &str)]) -> String {
        let mut items: Vec<String> = (0..MIN_EXPECTED_GAMES as u64)
            .map(|i| format!(r#"{{"id":"{}","name":"Filler {i}"}}"#, 100_000_000_000_000_000u64 + i))
            .collect();
        for (id, name) in extra {
            items.push(format!(r#"{{"id":"{id}","name":"{name}"}}"#));
        }
        format!("[{}]", items.join(","))
    }

    #[test]
    fn parses_the_discord_shape_and_ignores_extra_fields() {
        let mut raw = padded(&[]);
        raw.pop();
        raw.push_str(r#",{"id":"1402418491272986635","name":"Minecraft","icon":"abc","executables":[{"name":"javaw.exe"}],"hook":true}]"#);
        let games = parse_games(&raw).unwrap();
        assert!(games.iter().any(|g| g.name == "Minecraft" && g.id == "1402418491272986635"));
        assert_eq!(games.len(), MIN_EXPECTED_GAMES + 1);
    }

    #[test]
    fn drops_duplicates_blank_names_and_bad_ids() {
        let raw = padded(&[
            ("1402418491272986635", "Minecraft"),
            ("1402418491272986635", "Minecraft again"),
            ("1402418491272986636", "   "),
            ("not-a-number", "Bad id"),
            ("123", "Short id"),
        ]);
        let games = parse_games(&raw).unwrap();
        assert_eq!(games.len(), MIN_EXPECTED_GAMES + 1);
        assert_eq!(games.iter().filter(|g| g.id == "1402418491272986635").count(), 1);
    }

    #[test]
    fn entries_missing_fields_are_skipped() {
        let mut raw = padded(&[]);
        raw.pop();
        raw.push_str(r#",{"id":"1402418491272986635"},{"name":"No id"},{}]"#);
        assert_eq!(parse_games(&raw).unwrap().len(), MIN_EXPECTED_GAMES);
    }

    #[test]
    fn output_is_sorted_by_name_ignoring_case() {
        let raw = padded(&[
            ("200000000000000001", "zebra game"),
            ("200000000000000002", "Alpha game"),
            ("200000000000000003", "beta game"),
        ]);
        let games = parse_games(&raw).unwrap();
        let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(names, sorted);
    }

    #[test]
    fn suspiciously_small_responses_are_refused() {
        let err = parse_games(r#"[{"id":"1402418491272986635","name":"Minecraft"}]"#).unwrap_err();
        assert!(matches!(err, GamesError::TooFew(1)));
        assert!(parse_games("{}").is_err());
        assert!(parse_games("<html>rate limited</html>").is_err());
    }

    #[test]
    fn the_bundled_list_parses() {
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../shared/games.json"))
            .expect("shared/games.json");
        let list = GameList::from_json(&raw).unwrap();
        assert!(list.len() > 1000);
        assert!(list.search("minecraft", 5).iter().any(|g| g.name == "Minecraft"));
    }

    fn list() -> GameList {
        GameList::new(vec![
            game(1, "Apex Legends"),
            game(2, "Legends of Runeterra"),
            game(3, "League of Legends"),
            game(4, "Pokémon Go"),
            game(5, "Minecraft"),
            game(6, "Minecraft Dungeons"),
            game(7, "Terraria"),
            game(8, "The Legend of Zelda"),
            game(9, "Counter-Strike 2"),
        ])
    }

    #[test]
    fn empty_query_returns_the_first_n_alphabetically() {
        let l = list();
        let r = l.search("   ", 3);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].name, "Apex Legends");
    }

    #[test]
    fn exact_beats_prefix_beats_word_start_beats_substring() {
        let l = list();
        let r: Vec<String> = l.search("minecraft", 10).into_iter().map(|g| g.name).collect();
        assert_eq!(r, vec!["Minecraft", "Minecraft Dungeons"]);

        let r: Vec<String> = l.search("legends", 10).into_iter().map(|g| g.name).collect();
        // "Legends of Runeterra" starts with it, the others only have it as a later word
        assert_eq!(r[0], "Legends of Runeterra");
        assert!(r.contains(&"Apex Legends".to_string()));
        assert!(r.contains(&"League of Legends".to_string()));
    }

    #[test]
    fn matching_is_case_and_accent_insensitive() {
        let l = list();
        assert_eq!(l.search("POKEMON", 5)[0].name, "Pokémon Go");
        assert_eq!(l.search("pokémon", 5)[0].name, "Pokémon Go");
    }

    #[test]
    fn all_words_must_match_in_any_order() {
        let l = list();
        let r = l.search("legends league", 5);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "League of Legends");
        assert!(l.search("legends banana", 5).is_empty());
    }

    #[test]
    fn substrings_match_and_limit_applies() {
        let l = list();
        assert_eq!(l.search("raria", 5)[0].name, "Terraria");
        assert_eq!(l.search("e", 2).len(), 2);
        assert!(l.search("zzzz", 5).is_empty());
    }

    #[test]
    fn lookup_by_id() {
        let l = list();
        assert_eq!(l.get("5").unwrap().name, "Minecraft");
        assert!(l.get("nope").is_none());
    }
}
