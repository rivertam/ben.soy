//! One airport registry and Unicode-aware search for SSR and browser Wasm.

use std::collections::HashMap;
use std::sync::LazyLock;

use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Coordinates {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct Airport {
    pub iata: String,
    pub name: String,
    pub city: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
    /// Size proxy from runway data; higher = more significant airport.
    pub weight: i64,
}

impl Airport {
    pub fn coordinates(&self) -> Coordinates {
        Coordinates {
            lat: self.lat,
            lon: self.lon,
        }
    }
}

/// Multi-airport city groupings and alternate city names for search.
///
/// The OurAirports `municipality` field is the airport's literal town — EWR is
/// "Newark", IAD is "Dulles", NRT is "Narita" — so searching the city people
/// actually fly to would miss most of a metro area's airports. Each entry here
/// connects a city (and the names people type for it) to its airports.
///
/// `code` is the IATA metropolitan-area code (NYC, LON, TYO…) or, for cities
/// whose main airport shares its code (BKK, IST…), that shared city code.
/// Groupings follow the IATA city codes as flight-search engines use them. A
/// member with no scheduled service in the current dataset (e.g. KBP) is
/// ignored at index time, so entries may list airports ahead of the data.
struct Metro {
    /// IATA metropolitan-area or shared city code, when one exists.
    code: Option<&'static str>,
    /// Primary city name; searchable for every member airport.
    name: &'static str,
    /// Alternate names and abbreviations people type.
    aliases: &'static [&'static str],
    /// Member airports, by IATA code.
    airports: &'static [&'static str],
}

#[rustfmt::skip]
const METROS: &[Metro] = &[
    // North America
    Metro { code: Some("NYC"), name: "New York", aliases: &["New York City"], airports: &["JFK", "EWR", "LGA", "SWF"] },
    Metro { code: Some("WAS"), name: "Washington", aliases: &["Washington DC", "DC"], airports: &["IAD", "DCA", "BWI"] },
    Metro { code: Some("CHI"), name: "Chicago", aliases: &[], airports: &["ORD", "MDW"] },
    Metro { code: Some("HOU"), name: "Houston", aliases: &[], airports: &["IAH", "HOU"] },
    Metro { code: None, name: "Los Angeles", aliases: &["LA"], airports: &["LAX", "BUR", "LGB", "SNA", "ONT"] },
    Metro { code: None, name: "San Francisco", aliases: &["SF", "Bay Area"], airports: &["SFO", "OAK", "SJC"] },
    Metro { code: Some("YTO"), name: "Toronto", aliases: &[], airports: &["YYZ", "YTZ"] },
    Metro { code: Some("YMQ"), name: "Montreal", aliases: &[], airports: &["YUL"] },
    // Latin America
    Metro { code: Some("SAO"), name: "São Paulo", aliases: &[], airports: &["GRU", "CGH", "VCP"] },
    Metro { code: Some("RIO"), name: "Rio de Janeiro", aliases: &[], airports: &["GIG", "SDU"] },
    Metro { code: Some("BHZ"), name: "Belo Horizonte", aliases: &[], airports: &["CNF", "PLU"] },
    Metro { code: Some("BUE"), name: "Buenos Aires", aliases: &[], airports: &["EZE", "AEP"] },
    // Europe
    Metro { code: Some("LON"), name: "London", aliases: &[], airports: &["LHR", "LGW", "LCY", "STN", "LTN", "SEN"] },
    Metro { code: Some("PAR"), name: "Paris", aliases: &[], airports: &["CDG", "ORY", "BVA"] },
    Metro { code: Some("MIL"), name: "Milan", aliases: &["Milano"], airports: &["MXP", "LIN", "BGY"] },
    Metro { code: Some("ROM"), name: "Rome", aliases: &["Roma"], airports: &["FCO", "CIA"] },
    Metro { code: Some("VCE"), name: "Venice", aliases: &["Venezia"], airports: &["VCE", "TSF"] },
    Metro { code: None, name: "Florence", aliases: &[], airports: &["FLR"] }, // municipality is "Firenze"
    Metro { code: None, name: "Munich", aliases: &["München", "Muenchen"], airports: &["MUC"] },
    Metro { code: None, name: "Cologne", aliases: &["Koeln", "Bonn"], airports: &["CGN"] }, // municipality is "Köln"
    Metro { code: None, name: "Vienna", aliases: &["Wien"], airports: &["VIE"] },
    Metro { code: None, name: "Prague", aliases: &["Praha"], airports: &["PRG"] },
    Metro { code: Some("STO"), name: "Stockholm", aliases: &[], airports: &["ARN", "NYO", "BMA", "VST"] },
    Metro { code: Some("REK"), name: "Reykjavik", aliases: &[], airports: &["KEF", "RKV"] },
    Metro { code: Some("WAW"), name: "Warsaw", aliases: &["Warszawa"], airports: &["WAW", "WMI"] },
    Metro { code: Some("BUH"), name: "Bucharest", aliases: &["Bucuresti"], airports: &["OTP", "BBU"] },
    Metro { code: Some("MOW"), name: "Moscow", aliases: &["Moskva"], airports: &["SVO", "DME", "VKO", "ZIA"] },
    Metro { code: Some("IEV"), name: "Kyiv", aliases: &["Kiev"], airports: &["KBP", "IEV"] },
    Metro { code: Some("IST"), name: "Istanbul", aliases: &[], airports: &["IST", "SAW"] },
    Metro { code: Some("TCI"), name: "Tenerife", aliases: &[], airports: &["TFN", "TFS"] },
    // Middle East
    Metro { code: Some("DXB"), name: "Dubai", aliases: &[], airports: &["DXB", "DWC"] },
    Metro { code: Some("THR"), name: "Tehran", aliases: &[], airports: &["IKA", "THR"] },
    // Asia
    Metro { code: Some("TYO"), name: "Tokyo", aliases: &[], airports: &["HND", "NRT"] },
    Metro { code: Some("OSA"), name: "Osaka", aliases: &[], airports: &["KIX", "ITM", "UKB"] },
    Metro { code: Some("NGO"), name: "Nagoya", aliases: &[], airports: &["NGO", "NKM"] },
    Metro { code: Some("SPK"), name: "Sapporo", aliases: &[], airports: &["CTS", "OKD"] },
    Metro { code: Some("SEL"), name: "Seoul", aliases: &[], airports: &["ICN", "GMP"] },
    Metro { code: Some("BJS"), name: "Beijing", aliases: &["Peking"], airports: &["PEK", "PKX"] },
    Metro { code: Some("SHA"), name: "Shanghai", aliases: &[], airports: &["PVG", "SHA"] },
    Metro { code: Some("TPE"), name: "Taipei", aliases: &[], airports: &["TPE", "TSA"] },
    Metro { code: Some("JKT"), name: "Jakarta", aliases: &[], airports: &["CGK", "HLP"] },
    Metro { code: Some("BKK"), name: "Bangkok", aliases: &[], airports: &["BKK", "DMK"] },
    Metro { code: None, name: "Mumbai", aliases: &["Bombay"], airports: &["BOM"] },
    Metro { code: None, name: "Chennai", aliases: &["Madras"], airports: &["MAA"] },
    Metro { code: None, name: "Kolkata", aliases: &["Calcutta"], airports: &["CCU"] },
    Metro { code: None, name: "Ho Chi Minh City", aliases: &["Saigon"], airports: &["SGN"] },
    Metro { code: None, name: "Guangzhou", aliases: &["Canton"], airports: &["CAN"] },
    Metro { code: None, name: "Yangon", aliases: &["Rangoon"], airports: &["RGN"] },
    // Oceania
    Metro { code: Some("MEL"), name: "Melbourne", aliases: &[], airports: &["MEL", "AVV"] },
];

/// Preserve the browser's NFD + Diacritic removal and letter/number matching.
fn fold(s: &str) -> String {
    static STRIP: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"[\p{Diacritic}]|[^\p{L}\p{N}\s/-]")
            .expect("static Unicode search pattern")
    });
    let decomposed: String = s.to_lowercase().nfd().collect();
    STRIP.replace_all(&decomposed, "").into_owned()
}

/// JS `split(/[\s/-]+/)` semantics: runs of separators delimit, and a leading
/// or trailing run yields an empty piece (query tokens filter those out;
/// indexed word lists keep them, as the original does).
fn split_words(s: &str) -> Vec<String> {
    let mut words = vec![String::new()];
    let mut pending_sep = false;
    for c in s.chars() {
        if c.is_whitespace() || c == '/' || c == '-' {
            pending_sep = true;
        } else {
            if pending_sep {
                words.push(String::new());
                pending_sep = false;
            }
            words.last_mut().unwrap().push(c);
        }
    }
    if pending_sep {
        words.push(String::new());
    }
    words
}

struct Indexed {
    /// Position in `Db::airports` (and in `Db::index` — same order).
    airport: usize,
    city: String,
    city_words: Vec<Vec<char>>,
    country: String,
    country_words: Vec<Vec<char>>,
    name_words: Vec<Vec<char>>,
    /// The folded name words re-joined with single spaces (`nameWords.join(' ')`).
    name_joined: String,
    iata: String,
    /// Folded metro names/codes this airport answers to, e.g. "nyc", "new york".
    aliases: Vec<String>,
    alias_word_lists: Vec<Vec<Vec<char>>>,
}

struct Db {
    airports: Vec<Airport>,
    by_iata: HashMap<String, usize>,
    index: Vec<Indexed>,
}

fn word_chars(word: &str) -> Vec<char> {
    word.chars().collect()
}

static DB: LazyLock<Db> = LazyLock::new(|| {
    let airports: Vec<Airport> = serde_json::from_str(include_str!("../../../data/airports.json"))
        .expect("data/airports.json parses");
    let by_iata: HashMap<String, usize> = airports
        .iter()
        .enumerate()
        .map(|(i, a)| (a.iata.clone(), i))
        .collect();

    let mut index: Vec<Indexed> = airports
        .iter()
        .enumerate()
        .map(|(i, airport)| {
            let city = fold(&airport.city);
            let country = fold(&airport.country);
            let name_words = split_words(&fold(&airport.name));
            Indexed {
                airport: i,
                city_words: split_words(&city).iter().map(|w| word_chars(w)).collect(),
                country_words: split_words(&country)
                    .iter()
                    .map(|w| word_chars(w))
                    .collect(),
                name_joined: name_words.join(" "),
                name_words: name_words.iter().map(|w| word_chars(w)).collect(),
                city,
                country,
                iata: airport.iata.to_lowercase(),
                aliases: Vec::new(),
                alias_word_lists: Vec::new(),
            }
        })
        .collect();

    let index_by_iata: HashMap<String, usize> = index
        .iter()
        .enumerate()
        .map(|(i, e)| (e.iata.clone(), i))
        .collect();
    for metro in METROS {
        let mut aliases: Vec<&str> = Vec::with_capacity(metro.aliases.len() + 2);
        aliases.push(metro.name);
        aliases.extend_from_slice(metro.aliases);
        if let Some(code) = metro.code {
            aliases.push(code);
        }
        let folded: Vec<String> = aliases.iter().map(|a| fold(a)).collect();
        let word_lists: Vec<Vec<Vec<char>>> = folded
            .iter()
            .map(|a| split_words(a).iter().map(|w| word_chars(w)).collect())
            .collect();
        for code in metro.airports {
            let Some(&e) = index_by_iata.get(&code.to_lowercase()) else {
                continue; // no scheduled service in the current dataset
            };
            index[e].aliases.extend(folded.iter().cloned());
            index[e].alias_word_lists.extend(word_lists.iter().cloned());
        }
    }

    Db {
        airports,
        by_iata,
        index,
    }
});

pub fn find_airport(iata: &str) -> Option<&'static Airport> {
    let db = LazyLock::force(&DB);
    db.by_iata
        .get(&iata.to_uppercase())
        .map(|&i| &db.airports[i])
}

/// True when `word` is within one edit (insert/delete/replace/adjacent swap) of `token`.
fn within_one_edit(token: &[char], word: &[char]) -> bool {
    if token.len().abs_diff(word.len()) > 1 {
        return false;
    }
    let mut i = 0;
    while i < token.len() && i < word.len() && token[i] == word[i] {
        i += 1;
    }
    if i == token.len() && i == word.len() {
        return true;
    }
    if token.len() == word.len() {
        if token.get(i) == word.get(i + 1) && token.get(i + 1) == word.get(i) {
            let tail = (i + 2).min(token.len());
            return token[tail..] == word[tail..];
        }
        return token[i + 1..] == word[i + 1..];
    }
    let (shorter, longer) = if token.len() < word.len() {
        (token, word)
    } else {
        (word, token)
    };
    shorter[i..] == longer[i + 1..]
}

struct Token {
    text: String,
    chars: Vec<char>,
}

/// Every query token must prefix-match (or, for tokens of 4+ chars, fuzzily match) a distinct word.
/// `Some(exact)` on a match, `None` otherwise.
fn tokens_match_words(tokens: &[Token], words: &[Vec<char>]) -> Option<bool> {
    let mut used = vec![false; words.len()];
    let mut exact = true;
    for token in tokens {
        let mut found = None;
        for (w, word) in words.iter().enumerate() {
            if !used[w] && word.starts_with(&token.chars) {
                found = Some(w);
                break;
            }
        }
        if found.is_none() && token.chars.len() >= 4 {
            for (w, word) in words.iter().enumerate() {
                if !used[w]
                    && within_one_edit(&token.chars, &word[..token.chars.len().min(word.len())])
                {
                    found = Some(w);
                    exact = false;
                    break;
                }
            }
        }
        used[found?] = true;
    }
    Some(exact)
}

fn match_quality(entry: &Indexed, q: &str, tokens: &[Token]) -> i32 {
    if entry.iata == q {
        return 100;
    }
    let mut quality = 0;
    // A full metro name/code ("nyc", "washington dc") beats partial-IATA and
    // plain city matches but never an exactly typed airport code.
    if entry.aliases.iter().any(|a| a == q) {
        quality = 92;
    } else if entry.iata.starts_with(q) {
        quality = 70;
    }

    if let Some(exact) = tokens_match_words(tokens, &entry.city_words) {
        let base = if entry.city.starts_with(&tokens[0].text) {
            70
        } else {
            62
        };
        quality = quality.max(if exact { base } else { base - 30 });
    }
    for words in &entry.alias_word_lists {
        if let Some(exact) = tokens_match_words(tokens, words) {
            let base = if words[0].starts_with(&tokens[0].chars) {
                70
            } else {
                62
            };
            quality = quality.max(if exact { base } else { base - 30 });
        }
    }
    if let Some(exact) = tokens_match_words(tokens, &entry.name_words) {
        quality = quality.max(if exact { 55 } else { 28 });
    }
    // Country is below city/metro/name so "Paris" still prefers the city, but
    // above the substring fallback and fuzzy name noise ("Japan" → GEO/Jagan).
    // how-bad never searched country; this is an intentional extension.
    if let Some(exact) = tokens_match_words(tokens, &entry.country_words) {
        let base = if entry.country.starts_with(&tokens[0].text) {
            45
        } else {
            38
        };
        quality = quality.max(if exact { base } else { base - 15 });
    }

    if quality == 0
        && q.chars().count() >= 3
        && (entry.city.contains(q) || entry.name_joined.contains(q) || entry.country.contains(q))
    {
        quality = 20;
    }
    quality
}

/// Search the shared registry, returning at most `limit` ranked results.
pub fn search_airports(query: &str, limit: usize) -> Vec<&'static Airport> {
    let db = LazyLock::force(&DB);
    let q = fold(query.trim());
    if q.is_empty() {
        return Vec::new();
    }
    let tokens: Vec<Token> = split_words(&q)
        .into_iter()
        .filter(|t| !t.is_empty())
        .map(|text| Token {
            chars: text.chars().collect(),
            text,
        })
        .collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(usize, i32)> = Vec::new();
    for entry in &db.index {
        let quality = match_quality(entry, &q, &tokens);
        if quality > 0 {
            scored.push((entry.airport, quality));
        }
    }

    // Tie-break on the folded city name — a primary-strength stand-in for the
    // original's `localeCompare` (both sorts are stable).
    scored.sort_by(|&(x, xq), &(y, yq)| {
        yq.cmp(&xq)
            .then_with(|| db.airports[y].weight.cmp(&db.airports[x].weight))
            .then_with(|| db.index[x].city.cmp(&db.index[y].city))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(i, _)| &db.airports[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(q: &str) -> Vec<&'static str> {
        search_airports(q, 8)
            .iter()
            .map(|a| a.iata.as_str())
            .collect()
    }

    #[test]
    fn unicode_names_match_composed_decomposed_and_unaccented_input() {
        for query in ["Cần Thơ", "Can Tho", "Ca\u{0302}\u{0300}n Tho\u{031b}"] {
            assert_eq!(codes(query)[0], "VCA", "{query}");
        }
        assert_eq!(codes("São Paulo"), codes("Sao Paulo"));
        assert_eq!(fold("Hawaiʻi"), "hawaii");
    }

    #[test]
    fn find_airport_is_case_insensitive() {
        assert!(find_airport("JFK").is_some());
        assert!(find_airport("jfk").is_some());
        assert!(find_airport("ZZZ").is_none());
        assert!(find_airport("").is_none());
    }

    #[test]
    fn search_finds_by_city_and_code() {
        assert!(!search_airports("new york", 5).is_empty());
        assert!(!search_airports("LHR", 5).is_empty());
        assert!(search_airports("zzzzzzzz", 5).is_empty());
    }

    #[test]
    fn nyc_lists_new_york_airports_biggest_first() {
        assert_eq!(&codes("NYC")[..3], ["JFK", "EWR", "LGA"]);
        assert!(codes("nyc").contains(&"SWF"));
    }

    #[test]
    fn metro_city_name_reaches_airports_whose_municipality_differs() {
        assert!(codes("new york").contains(&"EWR"));
        assert_eq!(&codes("tokyo")[..2], ["HND", "NRT"]);
        assert_eq!(&codes("washington")[..3], ["IAD", "BWI", "DCA"]);
    }

    #[test]
    fn metro_codes_and_abbreviations_resolve() {
        assert_eq!(&codes("dc")[..3], ["IAD", "BWI", "DCA"]);
        assert_eq!(codes("la")[0], "LAX");
        assert_eq!(codes("ny")[0], "JFK");
    }

    #[test]
    fn aliases_map_old_and_local_names() {
        assert_eq!(codes("saigon")[0], "SGN");
        assert_eq!(codes("bombay")[0], "BOM");
        assert_eq!(codes("münchen")[0], "MUC");
    }

    #[test]
    fn country_name_lists_that_countrys_airports() {
        let japan = codes("Japan");
        assert!(japan.contains(&"HND"), "{japan:?}");
        assert!(japan.contains(&"NRT"), "{japan:?}");
        assert!(japan.contains(&"KIX"), "{japan:?}");
        assert!(!japan.contains(&"GEO"), "fuzzy Jagan must not beat Japan");

        let top = search_airports("Japan", 8);
        assert!(top.iter().all(|a| a.country == "Japan"), "{top:?}");
    }

    #[test]
    fn separator_only_queries_return_nothing() {
        assert!(search_airports("-", 8).is_empty());
        assert!(search_airports("...", 8).is_empty());
    }
}
