//! `browser::find-in-page` — find in page, the way a browser's find bar works: the
//! matches are highlighted in the live document (CSS Highlight API, so the
//! page's DOM is untouched), the current one is scrolled into view, and
//! next / previous step through them. `close` clears the highlights.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindInput {
    pub session_id: String,
    /// Text to look for. Empty with `action: close` clears the search.
    #[serde(default)]
    pub query: String,
    /// `search` (default: run the query from the top), `next`, `previous`,
    /// or `close`.
    #[serde(default)]
    pub action: Option<String>,
    /// Match case. Default false.
    #[serde(default)]
    pub case_sensitive: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FindOutput {
    pub ok: bool,
    /// Number of matches in the visible text of the page.
    pub count: u64,
    /// 1-based index of the highlighted match; 0 when there is none.
    pub index: u64,
    /// The query the counts refer to.
    pub query: String,
}

/// Injected search. The state lives on `window.__iiiFind` so next / previous
/// do not rescan; a changed query or an explicit `search` rescans. Script,
/// style and hidden subtrees are skipped.
pub fn find_script(query: &str, action: &str, case_sensitive: bool) -> String {
    let query = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".to_string());
    let action = serde_json::to_string(action).unwrap_or_else(|_| "\"search\"".to_string());
    format!(
        r#"(() => {{
  const q = {query};
  const action = {action};
  const cs = {case_sensitive};
  const S = window.__iiiFind || (window.__iiiFind = {{ query: '', cs: false, ranges: [], index: -1 }});
  const clear = () => {{
    try {{ CSS.highlights.delete('iii-find'); CSS.highlights.delete('iii-find-current'); }} catch (e) {{}}
    S.ranges = []; S.index = -1; S.query = '';
  }};
  if (action === 'close' || q === '') {{ clear(); return {{ count: 0, index: 0 }}; }}
  if (!document.getElementById('iii-find-style')) {{
    const st = document.createElement('style');
    st.id = 'iii-find-style';
    st.textContent = '::highlight(iii-find){{background:#ffe066;color:#111}}::highlight(iii-find-current){{background:#ff9b00;color:#111}}';
    (document.head || document.documentElement).appendChild(st);
  }}
  if (action === 'search' || S.query !== q || S.cs !== cs) {{
    S.ranges = []; S.query = q; S.cs = cs;
    const root = document.body || document.documentElement;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {{
      acceptNode: (n) => {{
        const p = n.parentElement;
        if (!p) return NodeFilter.FILTER_REJECT;
        const tag = p.tagName;
        if (tag === 'SCRIPT' || tag === 'STYLE' || tag === 'NOSCRIPT' || tag === 'TEMPLATE') return NodeFilter.FILTER_REJECT;
        if (p.id && p.id.startsWith('iii-')) return NodeFilter.FILTER_REJECT;
        if (typeof p.checkVisibility === 'function' && !p.checkVisibility()) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      }},
    }});
    const needle = cs ? q : q.toLowerCase();
    let n;
    while ((n = walker.nextNode())) {{
      const text = cs ? n.data : n.data.toLowerCase();
      let i = text.indexOf(needle);
      while (i >= 0) {{
        const r = new Range();
        r.setStart(n, i);
        r.setEnd(n, i + q.length);
        S.ranges.push(r);
        i = text.indexOf(needle, i + q.length);
      }}
    }}
    S.index = S.ranges.length ? 0 : -1;
  }} else if (S.ranges.length) {{
    const len = S.ranges.length;
    S.index = action === 'previous' ? (S.index - 1 + len) % len : (S.index + 1) % len;
  }}
  try {{
    CSS.highlights.set('iii-find', new Highlight(...S.ranges));
    if (S.index >= 0) CSS.highlights.set('iii-find-current', new Highlight(S.ranges[S.index]));
    else CSS.highlights.delete('iii-find-current');
  }} catch (e) {{}}
  if (S.index >= 0) {{
    const el = S.ranges[S.index].startContainer.parentElement;
    if (el) el.scrollIntoView({{ block: 'center', inline: 'nearest' }});
  }}
  return {{ count: S.ranges.length, index: S.index + 1 }};
}})()"#
    )
}
