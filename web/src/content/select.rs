//! Content selection via source-offset splicing. `remove_excluded` deletes the
//! source ranges of excluded subtrees; `scope_to_targets` keeps only matched
//! regions. Selectors use the tl subset (tag/.class/#id). tag boundaries fall
//! on ASCII `<`/`>`, so byte-offset slicing is always on char boundaries.

use tl::{parse, ParserOptions};

/// Splice out the given byte ranges (inclusive start, end) from `html`.
/// Ranges are sorted and merged before removal.
pub(crate) fn splice_out_ranges(html: &str, mut ranges: Vec<(usize, usize)>) -> String {
    if ranges.is_empty() {
        return html.to_string();
    }
    ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in ranges {
        match merged.last_mut() {
            Some(last) if s <= last.1.saturating_add(1) => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;
    for (s, e) in merged {
        if s > cursor {
            out.push_str(&html[cursor..s]);
        }
        cursor = e + 1; // end is inclusive
    }
    if cursor < html.len() {
        out.push_str(&html[cursor..]);
    }
    out
}

/// Delete every subtree matching any `excluded` selector. Returns `html`
/// unchanged when `excluded` is empty, parsing fails, or nothing matches.
pub fn remove_excluded(html: &str, excluded: &[String]) -> String {
    if excluded.is_empty() {
        return html.to_string();
    }
    let Ok(dom) = parse(html, ParserOptions::default()) else {
        return html.to_string();
    };
    let parser = dom.parser();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for sel in excluded {
        if let Some(iter) = dom.query_selector(sel.trim()) {
            for handle in iter {
                if let Some(tag) = handle.get(parser).and_then(|n| n.as_tag()) {
                    ranges.push(tag.boundaries(parser)); // (start, end) inclusive
                }
            }
        }
    }
    if ranges.is_empty() {
        return html.to_string();
    }
    splice_out_ranges(html, ranges)
}

/// Keep only the source of elements matching any `targets` selector (outermost
/// match wins; nested matches are dropped). `None` when `targets` is empty or
/// nothing matches — the caller then keeps the unscoped HTML.
pub fn scope_to_targets(html: &str, targets: &[String]) -> Option<String> {
    if targets.is_empty() {
        return None;
    }
    let dom = parse(html, ParserOptions::default()).ok()?;
    let parser = dom.parser();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for sel in targets {
        if let Some(iter) = dom.query_selector(sel.trim()) {
            for handle in iter {
                if let Some(tag) = handle.get(parser).and_then(|n| n.as_tag()) {
                    ranges.push(tag.boundaries(parser));
                }
            }
        }
    }
    if ranges.is_empty() {
        return None;
    }
    ranges.sort_by_key(|r| r.0);
    let mut kept: Vec<(usize, usize)> = Vec::new();
    for (s, e) in ranges {
        if let Some(last) = kept.last() {
            if s <= last.1 {
                continue; // contained in / overlapping a kept outer range
            }
        }
        kept.push((s, e));
    }
    let mut out = String::new();
    for (s, e) in kept {
        out.push_str(&html[s..=e]);
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn removes_excluded_tag() {
        let html = "<body><nav>menu</nav><p>keep me</p><footer>foot</footer></body>";
        let out = remove_excluded(html, &s(&["nav", "footer"]));
        assert!(out.contains("keep me"));
        assert!(!out.contains("menu"));
        assert!(!out.contains("foot"));
    }

    #[test]
    fn remove_excluded_noop_when_empty() {
        let html = "<p>x</p>";
        assert_eq!(remove_excluded(html, &[]), html);
    }

    #[test]
    fn scopes_to_target() {
        let html = "<body><nav>menu</nav><article>real content</article></body>";
        let out = scope_to_targets(html, &s(&["article"])).unwrap();
        assert!(out.contains("real content"));
        assert!(!out.contains("menu"));
    }

    #[test]
    fn scope_none_when_no_match() {
        assert!(scope_to_targets("<p>x</p>", &s(&["article"])).is_none());
    }
}
