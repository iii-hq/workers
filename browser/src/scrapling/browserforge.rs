//! Frozen BrowserForge 1.2.4 header generator used by Scrapling 0.4.9.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;
use serde_json::{Map, Value};

const MISSING: &str = "*MISSING_VALUE*";
const INPUT_JSON: &str = include_str!("../../vendor/browserforge-1.2.4/input-network.json");
const HEADER_JSON: &str = include_str!("../../vendor/browserforge-1.2.4/header-network.json");

const CHROME_ORDER: &[&str] = &[
    "Host",
    "Connection",
    "Content-Length",
    "Cache-Control",
    "sec-ch-ua",
    "sec-ch-ua-mobile",
    "sec-ch-ua-platform",
    "Origin",
    "Content-Type",
    "Upgrade-Insecure-Requests",
    "User-Agent",
    "Accept",
    "Sec-Fetch-Site",
    "Sec-Fetch-Mode",
    "Sec-Fetch-User",
    "Sec-Fetch-Dest",
    "Referer",
    "Accept-Encoding",
    "Accept-Language",
    "Cookie",
    ":method",
    ":authority",
    ":scheme",
    ":path",
    "content-length",
    "cache-control",
    "sec-ch-ua",
    "sec-ch-ua-mobile",
    "sec-ch-ua-platform",
    "origin",
    "content-type",
    "upgrade-insecure-requests",
    "user-agent",
    "accept",
    "sec-fetch-site",
    "sec-fetch-mode",
    "sec-fetch-user",
    "sec-fetch-dest",
    "referer",
    "accept-encoding",
    "accept-language",
    "cookie",
    "priority",
];

const FIREFOX_ORDER: &[&str] = &[
    "Host",
    "User-Agent",
    "Accept",
    "Accept-Language",
    "Accept-Encoding",
    "Content-Type",
    "Content-Length",
    "Origin",
    "Connection",
    "Referer",
    "Cookie",
    "Upgrade-Insecure-Requests",
    "Sec-Fetch-Dest",
    "Sec-Fetch-Mode",
    "Sec-Fetch-Site",
    "Sec-Fetch-User",
    "Priority",
    ":method",
    ":path",
    ":authority",
    ":scheme",
    "user-agent",
    "accept",
    "accept-language",
    "accept-encoding",
    "content-type",
    "content-length",
    "origin",
    "referer",
    "cookie",
    "upgrade-insecure-requests",
    "sec-fetch-dest",
    "sec-fetch-mode",
    "sec-fetch-site",
    "sec-fetch-user",
    "priority",
    "te",
];

#[derive(Deserialize)]
struct Network {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    name: String,
    #[serde(rename = "parentNames")]
    parent_names: Vec<String>,
    #[serde(rename = "possibleValues")]
    possible_values: Vec<String>,
    #[serde(rename = "conditionalProbabilities")]
    probabilities: Value,
}

#[derive(Clone, Copy)]
enum Profile {
    Http,
    Browser,
}

static INPUT: OnceLock<Result<Network, String>> = OnceLock::new();
static HEADERS: OnceLock<Result<Network, String>> = OnceLock::new();
static RNG: OnceLock<Mutex<PythonRandom>> = OnceLock::new();
static HTTP_DEFAULT_UA: OnceLock<Result<String, String>> = OnceLock::new();
static BROWSER_DEFAULTS: OnceLock<Result<(), String>> = OnceLock::new();

fn network(
    slot: &'static OnceLock<Result<Network, String>>,
    source: &'static str,
) -> Result<&'static Network, String> {
    match slot.get_or_init(|| serde_json::from_str(source).map_err(|error| error.to_string())) {
        Ok(network) => Ok(network),
        Err(error) => Err(error.clone()),
    }
}

fn input_network() -> Result<&'static Network, String> {
    network(&INPUT, INPUT_JSON)
}

fn header_network() -> Result<&'static Network, String> {
    network(&HEADERS, HEADER_JSON)
}

fn rng() -> &'static Mutex<PythonRandom> {
    RNG.get_or_init(|| Mutex::new(PythonRandom::from_os()))
}

fn lock_rng() -> std::sync::MutexGuard<'static, PythonRandom> {
    rng()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn default_user_agent() -> Result<String, String> {
    HTTP_DEFAULT_UA
        .get_or_init(|| {
            generate(Profile::Http, &mut lock_rng())?
                .into_iter()
                .find(|(name, _)| name == "User-Agent")
                .map(|(_, value)| value)
                .ok_or_else(|| "BrowserForge produced no User-Agent".to_string())
        })
        .clone()
}

pub fn generate_http_headers() -> Result<Vec<(String, String)>, String> {
    // Importing Scrapling's static engine initializes this cached value before
    // it generates per-request headers. The otherwise-unused draw is visible
    // when deterministic RNG fixtures make multiple requests.
    default_user_agent()?;
    generate(Profile::Http, &mut lock_rng())
}

pub fn initialize_browser_defaults() -> Result<(), String> {
    BROWSER_DEFAULTS
        .get_or_init(|| {
            // `_config_tools.py` imports fingerprints (which initializes the
            // HTTP default) and then generates Chromium and Chrome defaults.
            default_user_agent()?;
            let mut random = lock_rng();
            generate(Profile::Browser, &mut random)?;
            generate(Profile::Browser, &mut random)?;
            Ok(())
        })
        .clone()
}

pub(crate) fn randint(first: u32, last: u32) -> u32 {
    debug_assert!(first <= last);
    first + lock_rng().randbelow(last - first + 1)
}

fn generate(profile: Profile, random: &mut PythonRandom) -> Result<Vec<(String, String)>, String> {
    let input = input_network()?;
    let mut constraints = HashMap::<String, Vec<String>>::new();
    constraints.insert("*DEVICE".to_string(), vec!["desktop".to_string()]);
    constraints.insert(
        "*OPERATING_SYSTEM".to_string(),
        match profile {
            Profile::Http => vec!["windows", "macos", "linux"],
            Profile::Browser => vec!["linux"],
        }
        .into_iter()
        .map(str::to_string)
        .collect(),
    );
    let browser_values = &input
        .nodes
        .iter()
        .find(|node| node.name == "*BROWSER_HTTP")
        .ok_or("BrowserForge input network has no *BROWSER_HTTP node")?
        .possible_values;
    let browser_http = match profile {
        Profile::Browser => browser_values
            .iter()
            .filter(|value| browser_allowed(value, profile, "chrome"))
            .cloned()
            .collect(),
        Profile::Http => ["chrome", "firefox", "edge"]
            .into_iter()
            .flat_map(|name| {
                browser_values
                    .iter()
                    .filter(move |value| browser_allowed(value, profile, name))
                    .cloned()
            })
            .collect(),
    };
    constraints.insert("*BROWSER_HTTP".to_string(), browser_http);

    let mut sample = Map::new();
    if !consistent_sample(input, &constraints, &mut sample, 0, random) {
        return Err("No headers based on this input can be generated. Please relax or change some of the requirements you specified.".to_string());
    }
    generate_unrestricted(header_network()?, &mut sample, random)?;

    let browser_http = sample
        .get("*BROWSER_HTTP")
        .and_then(Value::as_str)
        .ok_or("BrowserForge sample has no *BROWSER_HTTP")?;
    let add_sec_fetch = browser_http
        .split_once('|')
        .and_then(|(browser, _)| browser.split_once('/').map(|(name, _)| name))
        .map(|name| matches!(name, "chrome" | "firefox" | "edge"))
        .ok_or("BrowserForge produced an invalid *BROWSER_HTTP")?;
    sample.insert(
        "accept-language".to_string(),
        Value::String("en-US;q=1.0".to_string()),
    );
    if add_sec_fetch {
        for (name, value) in [
            ("sec-fetch-mode", "same-site"),
            ("sec-fetch-dest", "navigate"),
            ("sec-fetch-site", "?1"),
            ("sec-fetch-user", "document"),
        ] {
            sample.insert(name.to_string(), Value::String(value.to_string()));
        }
    }

    let visible = sample
        .into_iter()
        .filter_map(|(name, value)| {
            let value = value.as_str()?.to_string();
            (!(name.eq_ignore_ascii_case("connection") && value == "close")
                && !name.starts_with('*')
                && value != MISSING)
                .then_some((name, value))
        })
        .collect::<HashMap<_, _>>();
    let user_agent = visible
        .get("User-Agent")
        .or_else(|| visible.get("user-agent"))
        .map(String::as_str)
        .ok_or("Failed to find User-Agent in generated response")?;
    // BrowserForge checks Chrome before Edge. Their order tables are equal for
    // the fields this frozen dataset emits, so preserve that quirk directly.
    let order = if user_agent.contains("Firefox") || user_agent.contains("FxiOS") {
        FIREFOX_ORDER
    } else if user_agent.contains("Chrome") || user_agent.contains("CriOS") {
        CHROME_ORDER
    } else {
        return Err("Failed to find browser in User-Agent".to_string());
    };
    let mut seen = HashSet::new();
    Ok(order
        .iter()
        .filter(|name| seen.insert(**name))
        .filter_map(|name| {
            visible
                .get(*name)
                .map(|value| (pascalize(name), value.clone()))
        })
        .collect())
}

fn browser_allowed(value: &str, profile: Profile, wanted: &str) -> bool {
    let Some((browser, http)) = value.split_once('|') else {
        return false;
    };
    if http != "2" {
        return false;
    }
    let Some((name, version)) = browser.split_once('/') else {
        return false;
    };
    let major = version
        .split('.')
        .next()
        .and_then(|value| value.parse::<u32>().ok());
    if name != wanted {
        return false;
    }
    match (profile, name, major) {
        (Profile::Browser, "chrome", Some(148)) => true,
        (Profile::Http, "chrome", Some(148)) => true,
        (Profile::Http, "firefox", Some(version)) => version >= 142,
        (Profile::Http, "edge", Some(version)) => version >= 140,
        _ => false,
    }
}

fn consistent_sample(
    network: &Network,
    constraints: &HashMap<String, Vec<String>>,
    sample: &mut Map<String, Value>,
    depth: usize,
    random: &mut PythonRandom,
) -> bool {
    if depth == network.nodes.len() {
        return true;
    }
    let node = &network.nodes[depth];
    let possibilities = constraints
        .get(&node.name)
        .map(Vec::as_slice)
        .unwrap_or(&node.possible_values);
    let mut banned = HashSet::new();
    loop {
        let Some(value) = sample_restricted(node, sample, possibilities, &banned, random) else {
            return false;
        };
        sample.insert(node.name.clone(), Value::String(value.clone()));
        if consistent_sample(network, constraints, sample, depth + 1, random) {
            return true;
        }
        banned.insert(value);
        sample.shift_remove(&node.name);
    }
}

fn generate_unrestricted(
    network: &Network,
    sample: &mut Map<String, Value>,
    random: &mut PythonRandom,
) -> Result<(), String> {
    for node in &network.nodes {
        if sample.contains_key(&node.name) {
            continue;
        }
        let probabilities = probability_table(node, sample)
            .ok_or_else(|| format!("BrowserForge has no probabilities for {}", node.name))?;
        let choices = probabilities.keys().map(String::as_str).collect::<Vec<_>>();
        let value = sample_value(&choices, probabilities, random)
            .ok_or_else(|| format!("BrowserForge has no value for {}", node.name))?;
        sample.insert(node.name.clone(), Value::String(value));
    }
    Ok(())
}

fn sample_restricted(
    node: &Node,
    sample: &Map<String, Value>,
    possibilities: &[String],
    banned: &HashSet<String>,
    random: &mut PythonRandom,
) -> Option<String> {
    let probabilities = probability_table(node, sample)?;
    let choices = possibilities
        .iter()
        .map(String::as_str)
        .filter(|value| !banned.contains(*value) && probabilities.contains_key(*value))
        .collect::<Vec<_>>();
    sample_value(&choices, probabilities, random)
}

fn probability_table<'a>(
    node: &'a Node,
    sample: &Map<String, Value>,
) -> Option<&'a Map<String, Value>> {
    let mut current = &node.probabilities;
    for parent in &node.parent_names {
        let table = current.as_object()?;
        let parent = sample.get(parent)?.as_str()?;
        current = table
            .get("deeper")
            .and_then(Value::as_object)
            .and_then(|deeper| deeper.get(parent))
            .or_else(|| table.get("skip"))?;
    }
    current.as_object()
}

fn sample_value(
    choices: &[&str],
    probabilities: &Map<String, Value>,
    random: &mut PythonRandom,
) -> Option<String> {
    let first = choices.first()?;
    let anchor = random.random();
    let mut cumulative = 0.0;
    for choice in choices {
        cumulative += probabilities.get(*choice)?.as_f64()?;
        if cumulative > anchor {
            return Some((*choice).to_string());
        }
    }
    Some((*first).to_string())
}

fn pascalize(name: &str) -> String {
    if name.starts_with(':') || name.starts_with("sec-ch-ua") {
        return name.to_string();
    }
    if matches!(name, "dnt" | "rtt" | "ect") {
        return name.to_ascii_uppercase();
    }
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| {
                    first
                        .to_uppercase()
                        .chain(chars.flat_map(char::to_lowercase))
                        .collect::<String>()
                })
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("-")
}

struct PythonRandom {
    state: [u32; 624],
    index: usize,
}

impl PythonRandom {
    fn from_os() -> Self {
        let mut bytes = [0u8; 624 * 4];
        if std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .is_ok()
        {
            let mut key = [0u32; 624];
            for (word, bytes) in key.iter_mut().zip(bytes.chunks_exact(4)) {
                *word = u32::from_ne_bytes(bytes.try_into().expect("four-byte chunk"));
            }
            return Self::from_key(&key);
        }
        let fallback = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u32;
        Self::from_key(&[fallback, std::process::id()])
    }

    #[cfg(test)]
    fn seeded(seed: u32) -> Self {
        Self::from_key(&[seed])
    }

    fn from_key(key: &[u32]) -> Self {
        let mut random = Self {
            state: [0; 624],
            index: 624,
        };
        random.init_genrand(19_650_218);
        let mut i = 1usize;
        let mut j = 0usize;
        for _ in 0..624.max(key.len()) {
            let previous = random.state[i - 1];
            random.state[i] = (random.state[i]
                ^ (previous ^ (previous >> 30)).wrapping_mul(1_664_525))
            .wrapping_add(key[j])
            .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                random.state[0] = random.state[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            let previous = random.state[i - 1];
            random.state[i] = (random.state[i]
                ^ (previous ^ (previous >> 30)).wrapping_mul(1_566_083_941))
            .wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                random.state[0] = random.state[623];
                i = 1;
            }
        }
        random.state[0] = 0x8000_0000;
        random.index = 624;
        random
    }

    fn init_genrand(&mut self, seed: u32) {
        self.state[0] = seed;
        for index in 1..624 {
            let previous = self.state[index - 1];
            self.state[index] = 1_812_433_253u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(index as u32);
        }
    }

    fn word(&mut self) -> u32 {
        if self.index >= 624 {
            for index in 0..624 {
                let y = (self.state[index] & 0x8000_0000)
                    | (self.state[(index + 1) % 624] & 0x7fff_ffff);
                self.state[index] = self.state[(index + 397) % 624]
                    ^ (y >> 1)
                    ^ if y & 1 == 0 { 0 } else { 0x9908_b0df };
            }
            self.index = 0;
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^= value >> 18;
        value
    }

    fn random(&mut self) -> f64 {
        let high = (self.word() >> 5) as u64;
        let low = (self.word() >> 6) as u64;
        ((high << 26) | low) as f64 / 9_007_199_254_740_992.0
    }

    fn getrandbits(&mut self, bits: u32) -> u32 {
        debug_assert!((1..=32).contains(&bits));
        self.word() >> (32 - bits)
    }

    fn randbelow(&mut self, upper: u32) -> u32 {
        debug_assert!(upper > 0);
        let bits = 32 - upper.leading_zeros();
        loop {
            let value = self.getrandbits(bits);
            if value < upper {
                return value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpython_seed_zero_and_frozen_header_sample_match() {
        let mut random = PythonRandom::seeded(0);
        assert_eq!(random.random(), 0.8444218515250481);
        let headers = generate(Profile::Http, &mut PythonRandom::seeded(0)).unwrap();
        assert_eq!(
            headers,
            vec![
                ("sec-ch-ua".into(), "\"Chromium\";v=\"148\", \"Google Chrome\";v=\"148\", \"Not/A)Brand\";v=\"99\"".into()),
                ("sec-ch-ua-mobile".into(), "?0".into()),
                ("sec-ch-ua-platform".into(), "\"macOS\"".into()),
                ("Upgrade-Insecure-Requests".into(), "1".into()),
                ("User-Agent".into(), "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36".into()),
                ("Accept".into(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7".into()),
                ("Sec-Fetch-Site".into(), "?1".into()),
                ("Sec-Fetch-Mode".into(), "same-site".into()),
                ("Sec-Fetch-User".into(), "document".into()),
                ("Sec-Fetch-Dest".into(), "navigate".into()),
                ("Accept-Encoding".into(), "gzip, deflate, br, zstd".into()),
                ("Accept-Language".into(), "en-US;q=1.0".into()),
            ]
        );
    }

    #[test]
    fn cpython_randint_click_sequence_matches() {
        let mut random = PythonRandom::seeded(0);
        assert_eq!(26 + random.randbelow(3), 27);
        assert_eq!(25 + random.randbelow(3), 26);
        assert_eq!(100 + random.randbelow(101), 105);
    }

    #[test]
    fn browser_profile_matches_frozen_linux_chromium() {
        let headers = generate(Profile::Browser, &mut PythonRandom::seeded(0)).unwrap();
        assert_eq!(
            headers
                .iter()
                .find(|(name, _)| name == "User-Agent")
                .map(|(_, value)| value.as_str()),
            Some("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36")
        );
    }

    #[test]
    fn ten_thousand_seed_header_corpus_matches_browserforge() {
        let mut hash = 14_695_981_039_346_656_037u64;
        for seed in 0..10_000 {
            for (name, value) in generate(Profile::Http, &mut PythonRandom::seeded(seed)).unwrap() {
                for byte in name.bytes().chain([0]).chain(value.bytes()).chain([255]) {
                    hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
                }
            }
        }
        assert_eq!(hash, 0xa96f_eabb_879e_a69c);
    }
}
