//! Content filters → filtered HTML (surviving blocks' source, concatenated).
//! Pruning scores block elements by tag weight, text/link density, and
//! boilerplate class/id penalties. BM25 ranks blocks against a query. Scoring
//! runs on `inner_text`, which recurses; the caller's depth guard
//! (`content::process`) prevents pathological nesting.

use std::collections::HashMap;

use rust_stemmers::{Algorithm, Stemmer};
use tl::{parse, ParserOptions};

use crate::schemas::{ContentFilter, FilterType, ThresholdType};

const BLOCK_TAGS: [&str; 19] = [
    "p",
    "div",
    "article",
    "section",
    "main",
    "li",
    "blockquote",
    "pre",
    "td",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "nav",
    "footer",
    "header",
    "aside",
];

// Matched against whole tokens of an element's class/id (split on non-alphanumeric),
// NOT as substrings — otherwise "ad" matches "mw-heading"/"header"/"thread" and
// short hints silently flag real content. Include full-word variants ("navigation",
// "navbox") so token-equality keeps the coverage substring matching used to give.
const BOILERPLATE_HINTS: [&str; 17] = [
    "nav",
    "navigation",
    "navbox",
    "footer",
    "sidebar",
    "comment",
    "comments",
    "menu",
    "ad",
    "ads",
    "promo",
    "social",
    "share",
    "related",
    "cookie",
    "banner",
    "breadcrumb",
];

pub struct Block {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub words: usize,
    pub link_density: f64,
    pub tag: String,
    pub boilerplate: bool,
}

fn tag_weight(t: &str) -> f64 {
    match t {
        "article" | "main" | "section" | "p" => 1.0,
        "blockquote" | "pre" | "li" | "td" => 0.8,
        "div" => 0.7,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => 0.6,
        "nav" | "footer" | "header" | "aside" => 0.5,
        _ => 0.5,
    }
}

/// Build the good-block set — `is_content_good` per block, OR a rescued content
/// heading (a heading that is not boilerplate and not inside a boilerplate
/// region) — then splice out every block whose span contains no good block.
/// Shared by pruning and BM25; they differ only in what counts as content-good.
fn keep_good(
    html: &str,
    blocks: &[Block],
    rescue_headings: bool,
    is_content_good: impl Fn(usize, &Block) -> bool,
) -> String {
    // Boilerplate spans, sorted by start, with a prefix-max of end → O(log n)
    // "is [start,end] inside any boilerplate block?" test (for heading rescue).
    let mut bp: Vec<(usize, usize)> = blocks
        .iter()
        .filter(|b| b.boilerplate)
        .map(|b| (b.start, b.end))
        .collect();
    bp.sort_unstable_by_key(|&(s, _)| s);
    let bp_starts: Vec<usize> = bp.iter().map(|&(s, _)| s).collect();
    let mut bp_max_end = vec![0usize; bp.len() + 1];
    for (i, &(_, e)) in bp.iter().enumerate() {
        bp_max_end[i + 1] = bp_max_end[i].max(e);
    }
    let inside_boilerplate = |start: usize, end: usize| -> bool {
        let k = bp_starts.partition_point(|&s| s <= start);
        k > 0 && bp_max_end[k] >= end
    };
    let is_heading = |t: &str| matches!(t, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");

    let mut good_starts: Vec<usize> = Vec::new();
    for (i, b) in blocks.iter().enumerate() {
        let heading_good = rescue_headings
            && is_heading(&b.tag)
            && !b.boilerplate
            && !inside_boilerplate(b.start, b.end);
        if is_content_good(i, b) || heading_good {
            good_starts.push(b.start);
        }
    }
    good_starts.sort_unstable();

    let mut removable: Vec<(usize, usize)> = Vec::new();
    for b in blocks {
        let lo = good_starts.partition_point(|&s| s < b.start);
        let hi = good_starts.partition_point(|&s| s <= b.end);
        if lo == hi {
            removable.push((b.start, b.end));
        }
    }
    crate::content::select::splice_out_ranges(html, removable)
}

/// Collect candidate block elements with the metrics needed for scoring.
/// Anchor text length is precomputed once and attributed to a block by source
/// containment, giving each block's link density.
pub fn collect_blocks(html: &str) -> Vec<Block> {
    let Ok(dom) = parse(html, ParserOptions::default()) else {
        return Vec::new();
    };
    let parser = dom.parser();

    // Anchors sorted by start for O(log n) attribution. `prefix` sums descendant
    // anchor text (anchor start inside the block); `end_max` (prefix-max of end)
    // detects an ancestor anchor wrapping the block. ponytail: well-nested HTML
    // lets start/end offsets alone decide descendant vs. ancestor containment.
    let mut anchors: Vec<(usize, usize, usize)> = Vec::new(); // (start, end, text_len)
    for node in dom.nodes() {
        if let Some(tag) = node.as_tag() {
            if tag.name().as_utf8_str().eq_ignore_ascii_case("a") {
                let (s, e) = tag.boundaries(parser);
                anchors.push((s, e, tag.inner_text(parser).chars().count()));
            }
        }
    }
    anchors.sort_unstable_by_key(|&(s, _, _)| s);
    let starts: Vec<usize> = anchors.iter().map(|&(s, _, _)| s).collect();
    let mut prefix = vec![0usize; anchors.len() + 1];
    let mut end_max = vec![0usize; anchors.len() + 1];
    for (i, &(_, e, len)) in anchors.iter().enumerate() {
        prefix[i + 1] = prefix[i] + len;
        end_max[i + 1] = end_max[i].max(e);
    }

    let mut blocks = Vec::new();
    for node in dom.nodes() {
        let Some(tag) = node.as_tag() else { continue };
        let name = tag.name().as_utf8_str().to_lowercase();
        if !BLOCK_TAGS.contains(&name.as_str()) {
            continue;
        }
        let (start, end) = tag.boundaries(parser);
        let text = tag.inner_text(parser);
        let text_chars = text.chars().count().max(1);
        let words = text.split_whitespace().count();
        let lo = starts.partition_point(|&s| s < start);
        let hi = starts.partition_point(|&s| s <= end);
        // A block fully enclosed by an ancestor <a> (valid HTML5) is entirely link
        // text, so score it as fully linked; otherwise attribute descendant anchors.
        let enclosing = starts.partition_point(|&s| s <= start);
        let link_density = if enclosing > 0 && end_max[enclosing] >= end {
            1.0
        } else {
            let link_chars = prefix[hi] - prefix[lo];
            (link_chars as f64 / text_chars as f64).min(1.0)
        };
        let attrs = tag.attributes();
        let class_id = format!(
            "{} {}",
            attrs.id().map(|b| b.as_utf8_str()).unwrap_or_default(),
            attrs.class().map(|b| b.as_utf8_str()).unwrap_or_default()
        )
        .to_lowercase();
        let boilerplate = matches!(name.as_str(), "nav" | "footer" | "aside")
            || class_id
                .split(|c: char| !c.is_alphanumeric())
                .any(|tok| BOILERPLATE_HINTS.contains(&tok));
        blocks.push(Block {
            start,
            end,
            text: text.into_owned(),
            words,
            link_density,
            tag: name,
            boilerplate,
        });
    }
    blocks
}

fn score(b: &Block) -> f64 {
    let penalty = if b.boilerplate { 0.5 } else { 1.0 };
    // Saturating density: rewards word-rich blocks, half-saturates at 15 words.
    let density = b.words as f64 / (b.words as f64 + 15.0);
    tag_weight(&b.tag) * penalty * (1.0 - b.link_density) * density
}

/// Remove boilerplate blocks from `html` in-place: score every block element,
/// mark low-scorers as removable (unless they contain a good block), splice
/// them out of the source. `dynamic` lowers the bar for high-weight tags.
pub fn prune(html: &str, threshold: f64, dynamic: bool, min_words: Option<u32>) -> String {
    let blocks = collect_blocks(html);
    // rescue_headings = true: pruning keeps document structure (section titles).
    keep_good(html, &blocks, true, |_, b| {
        let passes_words = min_words.is_none_or(|m| b.words as u32 >= m);
        let eff = if dynamic {
            threshold * (1.0 - 0.3 * (tag_weight(&b.tag) - 0.5))
        } else {
            threshold
        };
        passes_words && score(b) >= eff
    })
}

fn tokenize(text: &str, stemmer: &Stemmer) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .map(|t| stemmer.stem(t).into_owned())
        .collect()
}

/// Keep query-relevant content: score each block by BM25 (k1=1.2, b=0.75), treat
/// non-boilerplate blocks scoring >= `threshold` as good, and splice out subtrees
/// with no relevant block. Chrome (nav/footer/sidebar) is dropped even if it
/// matches the query. Empty query: no-op (returns `html`).
pub fn bm25(html: &str, query: &str, threshold: f64) -> String {
    if query.trim().is_empty() {
        return html.to_string();
    }
    let stemmer = Stemmer::create(Algorithm::English);
    let q = tokenize(query, &stemmer);
    if q.is_empty() {
        return html.to_string();
    }
    let blocks = collect_blocks(html);
    if blocks.is_empty() {
        return html.to_string();
    }

    let docs: Vec<Vec<String>> = blocks.iter().map(|b| tokenize(&b.text, &stemmer)).collect();
    let n = docs.len() as f64;
    let avgdl = docs.iter().map(|d| d.len()).sum::<usize>() as f64 / n;
    let (k1, b) = (1.2_f64, 0.75_f64);

    // Precompute tf and df once — O(D×T) instead of O(D²×Q×avgdl).
    let tf: Vec<HashMap<&str, usize>> = docs
        .iter()
        .map(|d| {
            let mut m: HashMap<&str, usize> = HashMap::new();
            for t in d {
                *m.entry(t.as_str()).or_insert(0) += 1;
            }
            m
        })
        .collect();
    let mut df: HashMap<&str, usize> = HashMap::new();
    for m in &tf {
        for &term in m.keys() {
            *df.entry(term).or_insert(0) += 1;
        }
    }

    let scores: Vec<f64> = docs
        .iter()
        .enumerate()
        .map(|(idx, doc)| {
            let dl = doc.len() as f64;
            let mut s = 0.0;
            for term in &q {
                let f = *tf[idx].get(term.as_str()).unwrap_or(&0) as f64;
                if f == 0.0 {
                    continue;
                }
                let d_f = *df.get(term.as_str()).unwrap_or(&0) as f64;
                let idf = (1.0 + (n - d_f + 0.5) / (d_f + 0.5)).ln();
                s += idf * (f * (k1 + 1.0)) / (f + k1 * (1.0 - b + b * dl / avgdl.max(1.0)));
            }
            s
        })
        .collect();

    // Removal-based: keep query-relevant PROSE, splice out everything else. A block
    // is relevant only if it scores >= threshold AND is not boilerplate AND is not
    // link-dominated (link_density < 0.5) — so a nav/sidebar that merely mentions a
    // query term (e.g. "Tools") is still dropped. Headings are NOT rescued here
    // (relevance, not structure). Old keep-outermost returned the whole wrapper.
    keep_good(html, &blocks, false, |i, b| {
        scores[i] >= threshold && !b.boilerplate && b.link_density < 0.5
    })
}

/// Dispatch to the configured filter, applying default thresholds.
/// `fallback_query` (page metadata) is used when the caller omits `query`.
pub fn apply(html: &str, cf: &ContentFilter, fallback_query: &str) -> String {
    let query = cf
        .query
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback_query);
    match cf.kind {
        FilterType::Pruning => {
            let threshold = cf.threshold.unwrap_or(0.48);
            let dynamic = matches!(cf.threshold_type, Some(ThresholdType::Dynamic));
            prune(html, threshold, dynamic, cf.min_word_threshold)
        }
        FilterType::Bm25 => {
            let threshold = cf.threshold.unwrap_or(1.0);
            bm25(html, query, threshold)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<body>
        <div class="nav"><a href="/a">Home</a> <a href="/b">About</a> <a href="/c">Blog</a></div>
        <article><p>The quick brown fox jumps over the lazy dog and keeps running through a long
        descriptive paragraph with plenty of real words and almost no links at all here.</p></article>
        <div class="footer">© 2026 <a href="/x">terms</a></div>
    </body>"#;

    const DOCS: &str = r#"<body>
        <p>Rust ownership and the borrow checker prevent data races at compile time.</p>
        <p>Bananas are a good source of potassium and make a tasty snack.</p>
        <p>The borrow checker enforces lifetimes so references never dangle in Rust.</p>
    </body>"#;

    #[test]
    fn bm25_keeps_query_relevant_blocks() {
        let out = bm25(DOCS, "rust borrow checker", 0.5);
        assert!(out.contains("borrow checker"));
        assert!(!out.contains("Bananas"));
    }

    #[test]
    fn bm25_scores_visible_text_not_markup() {
        // Query terms appear ONLY in markup (the href path), never in visible prose.
        // Scoring on inner_text must NOT rescue this block; scoring on raw HTML would.
        let html = r#"<body>
            <p><a href="/rust-borrow-checker-guide">read more</a> Bananas are a tasty snack rich
            in potassium enjoyed worldwide in smoothies and desserts by many people every day.</p>
            <p>Ownership and the borrow checker in Rust prevent data races at compile time via
            affine types and lifetimes the compiler verifies statically for every reference.</p>
        </body>"#;
        let out = bm25(html, "rust borrow checker", 0.5);
        assert!(out.contains("Ownership and the borrow checker")); // real prose kept
        assert!(!out.contains("Bananas")); // markup-only match dropped
    }

    #[test]
    fn bm25_empty_query_is_noop() {
        let out = bm25(DOCS, "", 1.0);
        assert!(out.contains("Bananas")); // nothing filtered
    }

    #[test]
    fn bm25_removes_irrelevant_and_boilerplate() {
        // Nested page: relevant section + irrelevant section + boilerplate nav/footer.
        // bm25 must drop nav/footer AND the irrelevant prose, keep the relevant prose.
        // Regression: old keep-outermost returned the whole query-matching wrapper.
        let html = r#"<body>
            <nav class="nav"><a href="/a">Home</a></nav>
            <div id="main">
                <p>Rust ownership and the borrow checker prevent data races at compile time
                through affine types and lifetimes the compiler verifies statically at build.</p>
                <p>Bananas are an excellent source of potassium and make a tasty snack in many
                smoothies and desserts enjoyed by people all around the world every single day.</p>
            </div>
            <footer class="footer">Privacy policy and cookie notice apply</footer>
        </body>"#;
        let out = bm25(html, "rust ownership borrow checker", 0.5);
        assert!(out.contains("borrow checker")); // relevant prose kept
        assert!(!out.contains("potassium")); // irrelevant prose removed
        assert!(!out.contains("Home")); // boilerplate nav removed
        assert!(!out.contains("Privacy policy")); // boilerplate footer removed
        assert!(out.len() < html.len());
    }

    #[test]
    fn apply_dispatches_pruning_and_bm25() {
        let pruned = apply(
            PAGE,
            &crate::schemas::ContentFilter {
                kind: crate::schemas::FilterType::Pruning,
                query: None,
                threshold: None,
                threshold_type: None,
                min_word_threshold: None,
            },
            "",
        );
        assert!(pruned.contains("quick brown fox"));

        let ranked = apply(
            DOCS,
            &crate::schemas::ContentFilter {
                kind: crate::schemas::FilterType::Bm25,
                query: Some("potassium snack".to_string()),
                threshold: None,
                threshold_type: None,
                min_word_threshold: None,
            },
            "",
        );
        assert!(ranked.contains("Bananas"));
        assert!(!ranked.contains("borrow checker"));
    }

    #[test]
    fn prune_keeps_article_drops_boilerplate() {
        let out = prune(PAGE, 0.48, false, None);
        assert!(out.contains("quick brown fox"));
        assert!(!out.contains("Home"));
        assert!(!out.contains("terms"));
        // Threshold genuinely matters: at 0.0 the nav block survives,
        // proving it was score-dropped at 0.48, not structurally excluded.
        assert!(prune(PAGE, 0.0, false, None).contains("Home"));
    }

    #[test]
    fn prune_respects_min_word_threshold() {
        let html = "<article><p>too short</p></article>";
        let out = prune(html, 0.0, false, Some(50));
        assert!(out.trim().is_empty());
    }

    #[test]
    fn prune_keeps_article_heading() {
        let html = r#"<article><h1>Pruning in Rust</h1>
        <p>The quick brown fox jumps over the lazy dog through a long and genuinely substantive
        paragraph of real prose with many ordinary words and very few links at all.</p></article>"#;
        let out = prune(html, 0.48, false, None);
        assert!(out.contains("Pruning in Rust")); // title rescued
        assert!(out.contains("quick brown fox"));
    }

    #[test]
    fn prune_keeps_heading_in_heading_classed_wrapper() {
        // Wikipedia wraps section headings as <div class="mw-heading"><h2>..</h2></div>.
        // Hint "ad" must NOT substring-match "heading" and flag the wrapper boilerplate,
        // which would drop every section heading. Regression for the substring bug.
        let html = r#"<article>
            <div class="mw-heading mw-heading2"><h2>History</h2></div>
            <p>The quick brown fox jumps over the lazy dog in a long substantive paragraph
            of genuine prose with many ordinary words and almost no links here at all.</p>
        </article>"#;
        let out = prune(html, 0.48, false, None);
        assert!(out.contains("History")); // section heading kept, not flagged via "ad"
        assert!(out.contains("quick brown fox"));
    }

    #[test]
    fn prune_drops_heading_inside_boilerplate() {
        let html = r#"<body>
        <footer class="footer"><h2>Quick Links</h2><a href="/a">A</a> <a href="/b">B</a></footer>
        <article><p>The quick brown fox jumps over the lazy dog through a long substantive paragraph
        of genuine article prose with many ordinary words and very few links here.</p></article>
    </body>"#;
        let out = prune(html, 0.48, false, None);
        assert!(out.contains("quick brown fox")); // content kept
        assert!(!out.contains("Quick Links")); // heading inside footer dropped
        assert!(!out.contains(">A<") && !out.contains("href=\"/a\"")); // footer links dropped
    }

    #[test]
    fn prune_drops_nested_boilerplate_in_wrapper() {
        let html = r#"<body><div id="page">
        <nav class="nav"><a href="/a">Home</a> <a href="/b">About</a> <a href="/c">Docs</a> <a href="/d">Blog</a></nav>
        <div id="main"><article><p>The quick brown fox jumps over the lazy dog while a long and
        genuinely substantive paragraph of real article prose continues with many ordinary words and
        very few links so that it reads as primary content rather than navigation chrome.</p></article></div>
        <footer class="footer">© 2026 Example Inc. <a href="/x">Terms</a> <a href="/y">Privacy</a></footer>
    </div></body>"#;
        let out = prune(html, 0.48, false, None);
        assert!(out.contains("quick brown fox")); // article kept
        assert!(!out.contains("Home")); // nested nav removed
        assert!(!out.contains("Terms")); // nested footer removed
                                         // and meaningfully shorter than the input
        assert!(out.len() < html.len());
    }

    #[test]
    fn prune_drops_block_wrapped_in_ancestor_anchor() {
        // Valid HTML5: <a> wrapping block content (a fully-linked "card"). The block
        // starts after the <a>, so start-based attribution alone sees zero link text
        // and scores the card as prose; the ancestor anchor must make it fully linked.
        let html = r#"<body>
            <a href="/card"><div class="card"><p>Read this featured teaser blurb that is long
            and wordy enough to look like genuine prose to the density scorer right here.</p></div></a>
            <article><p>The quick brown fox jumps over the lazy dog through a long substantive
            paragraph of genuine article prose with many ordinary words and very few links.</p></article>
        </body>"#;
        let out = prune(html, 0.48, false, None);
        assert!(out.contains("quick brown fox")); // real article kept
        assert!(!out.contains("featured teaser")); // fully-linked card dropped
    }
}
