//! Scrapling 0.4.9 Smart Element Tracking over the shared compatibility DOM.

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use rusqlite::{params, types::ValueRef, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Map;

use crate::scrapling::dom::{self, Doc, ElementRef};
use crate::scrapling::query::{self, QueryResult};

const DEFAULT_PATH: &str = "./data/scrapling/elements.db";
const MIN_SCORE: f64 = 40.0;

// Delta from psl 2.1.180 to tld 0.13.2's frozen 2026-03-06 PSL. The source
// snapshot's SHA-256 is
// abf32ce9987d505b89765d76f35760543851235508f1f426b5b259a2062b5f68.
const FROZEN_ADDED_EXACT: &str = "1cooldns.com
auth.cognito-idp.eusc-de-east-1.on.amazonwebservices.eu
blob.core.usgovcloudapi.net
bumbleshrimp.com
com.kh
corespeed.app
ddnsguru.com
discourse.diy
drive-platform.com
drive-platform.io
dynuddns.com
dynuddns.net
dynuhosting.com
edu.kh
eu-west-1.convex.cloud
eu-west-1.convex.site
file.core.usgovcloudapi.net
file.core.windows.net
gov.kh
hue.vn
imagine.diy
intouch.email
kdns.fr
keenetic.io
keenetic.link
keenetic.name
keenetic.pro
kh
miren.app
miren.systems
ms.fun
ms.show
my.be
mybox.company
mybox.me
mybox.page
mysynology.net
net.kh
opik.net
org.kh
pivohosting.com
roxa.org
s3-website.dualstack.us-gov-east-1.amazonaws.com
s3-website.dualstack.us-gov-west-1.amazonaws.com
sandbox.deno.net
shiptoday.app
shiptoday.build
sol.site
spawnbase.app
spryt.net
transfer-webapp.ap-southeast-7.on.aws
transfer-webapp.mx-central-1.on.aws
us-east-1.convex.cloud
us-east-1.convex.site
usgovtrafficmanager.net
web.core.usgovcloudapi.net
web.core.windows.net
wiredbladehosting.com";
const FROZEN_ADDED_WILDCARD_BASES: &str = "aa.crm.dev
ab.crm.dev
ac.crm.dev
ad.crm.dev
ae.crm.dev
af.crm.dev
begetcdn.cloud
ci.crm.dev
pa.crm.dev
pb.crm.dev
pc.crm.dev
pd.crm.dev
pe.crm.dev
pf.crm.dev";
const FROZEN_REMOVED_EXACT: &[&str] = &["12chars.dev", "12chars.it", "12chars.pro", "mazeplay.com"];

static STORAGE_PATH: OnceLock<RwLock<PathBuf>> = OnceLock::new();
// `u64::MAX` means oracle-compatible unbounded storage.
static MAX_BYTES: AtomicU64 = AtomicU64::new(u64::MAX);

fn configured_path() -> &'static RwLock<PathBuf> {
    STORAGE_PATH.get_or_init(|| RwLock::new(PathBuf::from(DEFAULT_PATH)))
}

/// Set the process-wide adaptive database path, matching the standalone
/// worker's one-time `storage.configure(...)` boot setting.
pub fn configure(path: impl AsRef<Path>) -> Result<(), String> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    *configured_path()
        .write()
        .map_err(|error| error.to_string())? = path.to_path_buf();
    Ok(())
}

/// Apply the live safe-mode storage ceiling. Compat passes `None` to retain
/// the standalone worker's unbounded behavior.
pub fn configure_quota(max_bytes: Option<u64>) {
    MAX_BYTES.store(max_bytes.unwrap_or(u64::MAX), Ordering::Relaxed);
}

fn storage_path() -> Result<PathBuf, String> {
    configured_path()
        .read()
        .map(|path| path.clone())
        .map_err(|error| error.to_string())
}

pub fn css_query<'a>(
    doc: &'a Doc,
    scope: Option<ElementRef<'a>>,
    selector: &str,
    domain: Option<&str>,
    identifier: &str,
    auto_save: bool,
) -> Result<Vec<QueryResult<'a>>, String> {
    let mut storage = Storage::open(domain)?;
    if selector.contains(',') {
        let selectors = cssselect::parse(selector)
            .map_err(|error| format!("Invalid CSS selector '{selector}': {error}"))?;
        let mut results = Vec::new();
        for parsed in selectors {
            let direct = query::css_query(doc, scope, &parsed.canonical())?;
            results.extend(storage.resolve(doc, direct, identifier, auto_save)?);
        }
        Ok(results)
    } else {
        let direct = query::css_query(doc, scope, selector)?;
        storage.resolve(doc, direct, identifier, auto_save)
    }
}

pub fn xpath_query<'a>(
    doc: &'a Doc,
    scope: Option<ElementRef<'a>>,
    selector: &str,
    domain: Option<&str>,
    identifier: &str,
    auto_save: bool,
) -> Result<Vec<QueryResult<'a>>, String> {
    let direct = crate::scrapling::xpath::xpath_query(doc, scope, selector)?;
    Storage::open(domain)?.resolve(doc, direct, identifier, auto_save)
}

struct Storage {
    connection: Connection,
    domain: String,
}

impl Storage {
    fn open(domain: Option<&str>) -> Result<Self, String> {
        let path = storage_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| error.to_string())?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS storage (\n\
                    id INTEGER PRIMARY KEY,\n\
                    url TEXT,\n\
                    identifier TEXT,\n\
                    element_data TEXT,\n\
                    UNIQUE (url, identifier)\n\
                );",
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection,
            domain: base_url(domain),
        })
    }

    fn resolve<'a>(
        &mut self,
        doc: &'a Doc,
        direct: Vec<QueryResult<'a>>,
        identifier: &str,
        auto_save: bool,
    ) -> Result<Vec<QueryResult<'a>>, String> {
        if !direct.is_empty() {
            if auto_save {
                self.save(result_element(&direct[0]), identifier)?;
            }
            return Ok(direct);
        }

        let Some(saved) = self.retrieve(identifier)? else {
            return Ok(Vec::new());
        };
        let relocated = relocate(doc, &saved);
        if auto_save {
            if let Some(first) = relocated.first().copied() {
                self.save(first, identifier)?;
            }
        }
        Ok(relocated.into_iter().map(QueryResult::Element).collect())
    }

    fn save(&mut self, element: ElementRef<'_>, identifier: &str) -> Result<(), String> {
        let bytes = serde_json::to_vec(&ElementData::from_element(element))
            .map_err(|error| error.to_string())?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO storage (url, identifier, element_data) VALUES (?, ?, ?)",
                params![self.domain, identifier, bytes],
            )
            .map_err(|error| error.to_string())?;
        let max_bytes = MAX_BYTES.load(Ordering::Relaxed);
        if max_bytes != u64::MAX {
            let pages: u64 = transaction
                .pragma_query_value(None, "page_count", |row| row.get(0))
                .map_err(|error| error.to_string())?;
            let page_size: u64 = transaction
                .pragma_query_value(None, "page_size", |row| row.get(0))
                .map_err(|error| error.to_string())?;
            if pages.saturating_mul(page_size) > max_bytes {
                return Err(format!(
                    "adaptive storage quota exceeded ({max_bytes} bytes); raise browser.scrapling.adaptive_max_bytes or disable adaptive mode"
                ));
            }
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    fn retrieve(&self, identifier: &str) -> Result<Option<ElementData>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT element_data FROM storage WHERE url = ? AND identifier = ?")
            .map_err(|error| error.to_string())?;
        let mut rows = statement
            .query(params![self.domain, identifier])
            .map_err(|error| error.to_string())?;
        let Some(row) = rows.next().map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        let value = row.get_ref(0).map_err(|error| error.to_string())?;
        let bytes = match value {
            ValueRef::Blob(bytes) | ValueRef::Text(bytes) => bytes,
            _ => return Err("adaptive element_data is neither BLOB nor TEXT".to_string()),
        };
        serde_json::from_slice(bytes)
            .map(Some)
            .map_err(|error| error.to_string())
    }
}

fn result_element<'a>(result: &QueryResult<'a>) -> ElementRef<'a> {
    match result {
        QueryResult::Element(element) => *element,
        QueryResult::Text { parent, .. } => *parent,
    }
}

fn base_url(domain: Option<&str>) -> String {
    let Some(raw) = domain.filter(|value| !value.is_empty()) else {
        return "default".to_string();
    };
    let lower = raw.to_lowercase();
    let parsed = url::Url::parse(&lower).or_else(|_| url::Url::parse(&format!("http://{lower}")));
    let Some(host) = parsed
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
    else {
        return "default".to_string();
    };
    frozen_registrable_domain(&host).unwrap_or_else(|| "default".to_string())
}

fn frozen_registrable_domain(host: &str) -> Option<String> {
    let exact = FROZEN_ADDED_EXACT
        .lines()
        .filter(|rule| host == *rule || host.ends_with(&format!(".{rule}")))
        .max_by_key(|rule| rule.len());
    let wildcard = FROZEN_ADDED_WILDCARD_BASES
        .lines()
        .filter_map(|base| {
            let prefix = host.strip_suffix(&format!(".{base}"))?;
            (!prefix.is_empty()).then_some((base, prefix))
        })
        .max_by_key(|(base, _)| base.len());

    let frozen_suffix = match (exact, wildcard) {
        (Some(rule), Some((base, prefix))) if base.len() + prefix.len() + 1 > rule.len() => {
            let wildcard_label = prefix.rsplit('.').next()?;
            format!("{wildcard_label}.{base}")
        }
        (Some(rule), _) => rule.to_string(),
        (None, Some((base, prefix))) => {
            let wildcard_label = prefix.rsplit('.').next()?;
            format!("{wildcard_label}.{base}")
        }
        (None, None) => String::new(),
    };
    if !frozen_suffix.is_empty() {
        return registrable_for_suffix(host, &frozen_suffix);
    }

    for rule in FROZEN_REMOVED_EXACT {
        if host == *rule || host.ends_with(&format!(".{rule}")) {
            return Some((*rule).to_string());
        }
    }
    if host == "goo"
        || host.ends_with(".goo")
        || host == "wolterskluwer"
        || host.ends_with(".wolterskluwer")
    {
        return None;
    }
    if host.ends_with(".cns.joyent.com") {
        return Some("joyent.com".to_string());
    }
    if let Some(prefix) = host.strip_suffix(".kh") {
        if !prefix.is_empty() {
            let label = prefix.rsplit('.').next()?;
            return Some(format!("{label}.kh"));
        }
    }

    let suffix = psl::suffix(host.as_bytes())?;
    if !suffix.is_known() {
        return None;
    }
    psl::domain_str(host)
        .or_else(|| std::str::from_utf8(suffix.trim().as_bytes()).ok())
        .map(str::to_string)
}

fn registrable_for_suffix(host: &str, suffix: &str) -> Option<String> {
    if host == suffix {
        return Some(suffix.to_string());
    }
    let prefix = host.strip_suffix(&format!(".{suffix}"))?;
    let label = prefix.rsplit('.').next()?;
    Some(format!("{label}.{suffix}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ElementData {
    tag: String,
    attributes: Map<String, serde_json::Value>,
    text: Option<String>,
    path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    parent_attribs: Map<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    siblings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<String>,
}

impl ElementData {
    fn from_element(element: ElementRef<'_>) -> Self {
        let attributes = element
            .attrs()
            .filter_map(|(name, value)| {
                let value = value.trim();
                (!value.is_empty()).then(|| {
                    (
                        name.to_string(),
                        serde_json::Value::String(value.to_string()),
                    )
                })
            })
            .collect();
        let raw_text = dom::leading_text(element);
        let text = (!raw_text.is_empty()).then(|| raw_text.trim().to_string());
        let mut path = vec![element.name().to_string()];
        let mut current = element;
        while let Some(parent) = dom::parent_element(current) {
            path.push(parent.name().to_string());
            current = parent;
        }
        path.reverse();

        let parent = dom::parent_element(element);
        let parent_name = parent.map(|value| value.name().to_string());
        let parent_attribs = parent
            .map(|value| dom::attrs_json(value))
            .unwrap_or_default();
        let parent_text = parent.and_then(|value| {
            let raw = dom::leading_text(value);
            (!raw.is_empty()).then(|| raw.trim().to_string())
        });
        let siblings = parent
            .map(|value| {
                dom::element_children(value)
                    .into_iter()
                    .filter(|child| child.id() != element.id())
                    .map(|child| child.name().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let children = dom::element_children(element)
            .into_iter()
            .map(|child| child.name().to_string())
            .collect();

        Self {
            tag: element.name().to_string(),
            attributes,
            text,
            path,
            parent_name,
            parent_attribs,
            parent_text,
            siblings,
            children,
        }
    }
}

fn relocate<'a>(doc: &'a Doc, saved: &ElementData) -> Vec<ElementRef<'a>> {
    let mut best = f64::NEG_INFINITY;
    let mut matches = Vec::new();
    for element in dom::descendant_elements(doc.root()) {
        let score = similarity(saved, &ElementData::from_element(element));
        if score > best {
            best = score;
            matches.clear();
            matches.push(element);
        } else if score == best {
            matches.push(element);
        }
    }
    if best >= MIN_SCORE {
        matches
    } else {
        Vec::new()
    }
}

fn similarity(original: &ElementData, candidate: &ElementData) -> f64 {
    let mut score = f64::from(original.tag == candidate.tag);
    let mut checks = 1usize;

    if let Some(text) = original.text.as_deref().filter(|value| !value.is_empty()) {
        score += string_ratio(text, candidate.text.as_deref().unwrap_or(""));
        checks += 1;
    }

    score += map_ratio(&original.attributes, &candidate.attributes);
    checks += 1;
    for name in ["class", "id", "href", "src"] {
        if let Some(value) =
            string_attr(&original.attributes, name).filter(|value| !value.is_empty())
        {
            score += string_ratio(
                value,
                string_attr(&candidate.attributes, name).unwrap_or(""),
            );
            checks += 1;
        }
    }

    score += sequence_ratio(&original.path, &candidate.path);
    checks += 1;

    if let Some(parent_name) = original
        .parent_name
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        if let Some(candidate_parent) = candidate.parent_name.as_deref() {
            score += string_ratio(parent_name, candidate_parent);
            checks += 1;
            score += map_ratio(&original.parent_attribs, &candidate.parent_attribs);
            checks += 1;
            if let Some(parent_text) = original
                .parent_text
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                score += string_ratio(parent_text, candidate.parent_text.as_deref().unwrap_or(""));
                checks += 1;
            }
        }
    }

    if !original.siblings.is_empty() {
        score += sequence_ratio(&original.siblings, &candidate.siblings);
        checks += 1;
    }

    round2((score / checks as f64) * 100.0)
}

fn string_attr<'a>(map: &'a Map<String, serde_json::Value>, name: &str) -> Option<&'a str> {
    map.get(name).and_then(serde_json::Value::as_str)
}

fn map_ratio(left: &Map<String, serde_json::Value>, right: &Map<String, serde_json::Value>) -> f64 {
    let left_keys: Vec<&str> = left.keys().map(String::as_str).collect();
    let right_keys: Vec<&str> = right.keys().map(String::as_str).collect();
    let left_values: Vec<&str> = left
        .values()
        .filter_map(serde_json::Value::as_str)
        .collect();
    let right_values: Vec<&str> = right
        .values()
        .filter_map(serde_json::Value::as_str)
        .collect();
    sequence_ratio(&left_keys, &right_keys) * 0.5
        + sequence_ratio(&left_values, &right_values) * 0.5
}

fn round2(value: f64) -> f64 {
    format!("{value:.2}").parse().expect("formatted f64")
}

fn string_ratio(left: &str, right: &str) -> f64 {
    sequence_ratio(
        &left.chars().collect::<Vec<_>>(),
        &right.chars().collect::<Vec<_>>(),
    )
}

fn sequence_ratio<T: Eq + Hash>(left: &[T], right: &[T]) -> f64 {
    let total = left.len() + right.len();
    if total == 0 {
        return 1.0;
    }
    let mut positions: HashMap<&T, Vec<usize>> = HashMap::new();
    for (index, item) in right.iter().enumerate() {
        positions.entry(item).or_default().push(index);
    }
    if right.len() >= 200 {
        let threshold = right.len() / 100 + 1;
        positions.retain(|_, indexes| indexes.len() <= threshold);
    }

    let mut queue = vec![(0, left.len(), 0, right.len())];
    let mut blocks = Vec::new();
    while let Some((left_start, left_end, right_start, right_end)) = queue.pop() {
        let (i, j, size) = longest_match(
            left,
            right,
            &positions,
            left_start,
            left_end,
            right_start,
            right_end,
        );
        if size == 0 {
            continue;
        }
        if left_start < i && right_start < j {
            queue.push((left_start, i, right_start, j));
        }
        if i + size < left_end && j + size < right_end {
            queue.push((i + size, left_end, j + size, right_end));
        }
        blocks.push((i, j, size));
    }
    blocks.sort_unstable();
    let matches: usize = blocks.into_iter().map(|(_, _, size)| size).sum();
    2.0 * matches as f64 / total as f64
}

#[allow(clippy::too_many_arguments)]
fn longest_match<T: Eq + Hash>(
    left: &[T],
    right: &[T],
    positions: &HashMap<&T, Vec<usize>>,
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> (usize, usize, usize) {
    let (mut best_i, mut best_j, mut best_size) = (left_start, right_start, 0usize);
    let mut previous = HashMap::new();
    for (i, item) in left.iter().enumerate().take(left_end).skip(left_start) {
        let mut current = HashMap::new();
        if let Some(indexes) = positions.get(item) {
            for &j in indexes {
                if j < right_start {
                    continue;
                }
                if j >= right_end {
                    break;
                }
                let size = if j == 0 {
                    1
                } else {
                    previous.get(&(j - 1)).copied().unwrap_or(0) + 1
                };
                current.insert(j, size);
                if size > best_size {
                    (best_i, best_j, best_size) = (i + 1 - size, j + 1 - size, size);
                }
            }
        }
        previous = current;
    }
    while best_i > left_start && best_j > right_start && left[best_i - 1] == right[best_j - 1] {
        best_i -= 1;
        best_j -= 1;
        best_size += 1;
    }
    while best_i + best_size < left_end
        && best_j + best_size < right_end
        && left[best_i + best_size] == right[best_j + best_size]
    {
        best_size += 1;
    }
    (best_i, best_j, best_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnlimitedOnDrop;

    impl Drop for UnlimitedOnDrop {
        fn drop(&mut self) {
            configure_quota(None);
        }
    }

    #[test]
    fn sequence_matcher_compares_unicode_code_points() {
        assert_eq!(string_ratio("é", "è"), 0.0);
    }

    #[test]
    fn sequence_matcher_does_not_extend_before_right_index_zero() {
        assert_eq!(sequence_ratio(b"xx", b"x"), 2.0 / 3.0);
    }

    #[test]
    fn safe_quota_rolls_back_an_oversized_save() {
        let path = std::env::temp_dir().join(format!(
            "browser-adaptive-quota-{}.db",
            uuid::Uuid::new_v4().simple()
        ));
        configure(&path).unwrap();
        configure_quota(Some(1));
        let _reset = UnlimitedOnDrop;
        let doc = dom::parse("<p id=x>saved identity</p>");
        let error = css_query(&doc, None, "p", Some("example.com"), "p", true).unwrap_err();
        assert!(error.contains("adaptive storage quota exceeded"), "{error}");

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
