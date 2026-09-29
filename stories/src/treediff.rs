//! Structural diff of two DOM captures (`runtime.js` `capture().dom`) and
//! the mapping from pixel regions to the elements under them. Nodes are
//! matched by position among their siblings with the same tag; a moved
//! node reads as removed + added.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::pixels::Region;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LayoutBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TreeChange {
    /// `div > button:nth-of-type(2) > span`, from the story content root.
    pub path: String,
    /// Nearest React component owning the element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// added | removed | text | attr | class | style | layout
    pub kind: String,
    /// The attribute, class list or style property that changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
    #[serde(default)]
    pub before: Value,
    #[serde(default)]
    pub after: Value,
    /// Layout box of the element in `b` (or `a` when removed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "box")]
    pub layout: Option<LayoutBox>,
}

fn layout_of(node: &Value) -> Option<LayoutBox> {
    let b = node.get("box")?;
    Some(LayoutBox {
        x: b.get("x")?.as_f64()?,
        y: b.get("y")?.as_f64()?,
        w: b.get("w")?.as_f64()?,
        h: b.get("h")?.as_f64()?,
    })
}

fn tag(node: &Value) -> &str {
    node.get("tag").and_then(Value::as_str).unwrap_or("?")
}

fn component(node: &Value) -> Option<String> {
    node.get("component")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn children(node: &Value) -> &[Value] {
    node.get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn empty_map() -> Map<String, Value> {
    Map::new()
}

fn object<'a>(node: &'a Value, key: &str) -> std::borrow::Cow<'a, Map<String, Value>> {
    match node.get(key).and_then(Value::as_object) {
        Some(map) => std::borrow::Cow::Borrowed(map),
        None => std::borrow::Cow::Owned(empty_map()),
    }
}

fn label(node: &Value, index_among_same_tag: usize) -> String {
    let t = tag(node);
    if index_among_same_tag == 0 {
        t.to_string()
    } else {
        format!("{t}:nth-of-type({})", index_among_same_tag + 1)
    }
}

fn child_paths(nodes: &[Value], parent: &str) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    nodes
        .iter()
        .map(|node| {
            let t = tag(node);
            let n = counts.entry(t).or_insert(0);
            let path = if parent.is_empty() {
                label(node, *n)
            } else {
                format!("{parent} > {}", label(node, *n))
            };
            *n += 1;
            (path, *n - 1)
        })
        .collect()
}

fn diff_nodes(a: &Value, b: &Value, path: &str, out: &mut Vec<TreeChange>) {
    let layout = layout_of(b);
    let comp = component(b).or_else(|| component(a));
    let push = |out: &mut Vec<TreeChange>,
                kind: &str,
                property: Option<String>,
                before: Value,
                after: Value| {
        out.push(TreeChange {
            path: path.to_string(),
            component: comp.clone(),
            kind: kind.into(),
            property,
            before,
            after,
            layout,
        });
    };
    let text = |node: &Value| node.get("text").cloned().unwrap_or(Value::Null);
    if text(a) != text(b) {
        push(out, "text", None, text(a), text(b));
    }
    let (ca, cb) = (
        a.get("classes").cloned().unwrap_or(Value::Array(vec![])),
        b.get("classes").cloned().unwrap_or(Value::Array(vec![])),
    );
    if ca != cb {
        push(out, "class", None, ca, cb);
    }
    let (aa, ab) = (object(a, "attrs"), object(b, "attrs"));
    for key in aa
        .keys()
        .chain(ab.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let (va, vb) = (
            aa.get(key).cloned().unwrap_or(Value::Null),
            ab.get(key).cloned().unwrap_or(Value::Null),
        );
        if va != vb {
            push(out, "attr", Some(key.clone()), va, vb);
        }
    }
    let (sa, sb) = (object(a, "styles"), object(b, "styles"));
    for key in sa
        .keys()
        .chain(sb.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let (va, vb) = (
            sa.get(key).cloned().unwrap_or(Value::Null),
            sb.get(key).cloned().unwrap_or(Value::Null),
        );
        if va != vb {
            push(out, "style", Some(key.clone()), va, vb);
        }
    }
    if let (Some(la), Some(lb)) = (layout_of(a), layout_of(b))
        && ((la.x - lb.x).abs() > 0.01
            || (la.y - lb.y).abs() > 0.01
            || (la.w - lb.w).abs() > 0.01
            || (la.h - lb.h).abs() > 0.01)
    {
        push(
            out,
            "layout",
            None,
            serde_json::to_value(la).unwrap_or(Value::Null),
            serde_json::to_value(lb).unwrap_or(Value::Null),
        );
    }
    diff_children(children(a), children(b), path, out);
}

fn diff_children(a: &[Value], b: &[Value], parent: &str, out: &mut Vec<TreeChange>) {
    let pa = child_paths(a, parent);
    let pb = child_paths(b, parent);
    let mut used = vec![false; a.len()];
    for (bi, nb) in b.iter().enumerate() {
        let wanted = &pb[bi].0;
        // Match by identical path (same tag and same index among that tag).
        let matched = pa
            .iter()
            .enumerate()
            .find(|(ai, (path, _))| path == wanted && !used[*ai])
            .map(|(ai, _)| ai);
        match matched {
            Some(ai) => {
                used[ai] = true;
                diff_nodes(&a[ai], nb, wanted, out);
            }
            None => out.push(TreeChange {
                path: wanted.clone(),
                component: component(nb),
                kind: "added".into(),
                property: None,
                before: Value::Null,
                after: summary(nb),
                layout: layout_of(nb),
            }),
        }
    }
    for (ai, na) in a.iter().enumerate() {
        if !used[ai] {
            out.push(TreeChange {
                path: pa[ai].0.clone(),
                component: component(na),
                kind: "removed".into(),
                property: None,
                before: summary(na),
                after: Value::Null,
                layout: layout_of(na),
            });
        }
    }
}

fn summary(node: &Value) -> Value {
    let mut map = Map::new();
    map.insert("tag".into(), Value::String(tag(node).into()));
    for key in ["classes", "attrs", "text"] {
        if let Some(value) = node.get(key) {
            map.insert(key.into(), value.clone());
        }
    }
    map.insert("children".into(), Value::from(children(node).len()));
    Value::Object(map)
}

/// Every change between two `capture().dom` arrays.
pub fn diff(a: &[Value], b: &[Value]) -> Vec<TreeChange> {
    let mut out = Vec::new();
    diff_children(a, b, "", &mut out);
    out
}

/// The deepest elements whose boxes contain the region's centre or overlap
/// at least half of the region. Paths only.
pub fn elements_for_region(dom: &[Value], region: &Region) -> Vec<String> {
    let (cx, cy) = (
        region.x as f64 + region.w as f64 / 2.0,
        region.y as f64 + region.h as f64 / 2.0,
    );
    let mut hits: Vec<(usize, String)> = Vec::new();
    fn walk(
        nodes: &[Value],
        parent: &str,
        depth: usize,
        cx: f64,
        cy: f64,
        region: &Region,
        hits: &mut Vec<(usize, String)>,
    ) {
        for (node, (path, _)) in nodes.iter().zip(child_paths(nodes, parent)) {
            if let Some(b) = layout_of(node) {
                let contains = cx >= b.x && cx <= b.x + b.w && cy >= b.y && cy <= b.y + b.h;
                let ox =
                    (b.x + b.w).min(region.x as f64 + region.w as f64) - b.x.max(region.x as f64);
                let oy =
                    (b.y + b.h).min(region.y as f64 + region.h as f64) - b.y.max(region.y as f64);
                let overlap = if ox > 0.0 && oy > 0.0 { ox * oy } else { 0.0 };
                let half = overlap >= 0.5 * (region.w as f64 * region.h as f64).max(1.0);
                if contains || half {
                    hits.push((depth, path.clone()));
                }
            }
            walk(children(node), &path, depth + 1, cx, cy, region, hits);
        }
    }
    walk(dom, "", 0, cx, cy, region, &mut hits);
    // Only the deepest level: ancestors are readable off the path itself.
    let deepest = hits.iter().map(|(d, _)| *d).max().unwrap_or(0);
    let mut out: Vec<String> = hits
        .into_iter()
        .filter(|(d, _)| *d == deepest)
        .map(|(_, p)| p)
        .collect();
    out.dedup();
    out.truncate(3);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(
        tag: &str,
        text: Option<&str>,
        styles: Value,
        children: Vec<Value>,
        bx: (f64, f64, f64, f64),
    ) -> Value {
        let mut v = json!({ "tag": tag, "styles": styles, "children": children, "box": { "x": bx.0, "y": bx.1, "w": bx.2, "h": bx.3 }, "component": "Button" });
        if let Some(t) = text {
            v["text"] = json!(t);
        }
        v
    }

    #[test]
    fn reports_text_style_layout_added_and_removed() {
        let a = vec![node(
            "div",
            None,
            json!({}),
            vec![
                node(
                    "button",
                    Some("Save"),
                    json!({ "font-weight": "500", "color": "rgb(0, 0, 0)" }),
                    vec![],
                    (0.0, 0.0, 60.0, 24.0),
                ),
                node(
                    "span",
                    Some("hint"),
                    json!({}),
                    vec![],
                    (70.0, 0.0, 30.0, 24.0),
                ),
            ],
            (0.0, 0.0, 100.0, 24.0),
        )];
        let b = vec![node(
            "div",
            None,
            json!({}),
            vec![
                node(
                    "button",
                    Some("Save"),
                    json!({ "font-weight": "600", "color": "rgb(0, 0, 0)" }),
                    vec![],
                    (0.0, 0.0, 64.0, 24.0),
                ),
                node(
                    "em",
                    Some("new"),
                    json!({}),
                    vec![],
                    (70.0, 0.0, 30.0, 24.0),
                ),
            ],
            (0.0, 0.0, 100.0, 24.0),
        )];
        let changes = diff(&a, &b);
        let kinds: Vec<(String, String)> = changes
            .iter()
            .map(|c| (c.path.clone(), c.kind.clone()))
            .collect();
        assert!(
            kinds.contains(&("div > button".into(), "style".into())),
            "{kinds:?}"
        );
        assert!(
            kinds.contains(&("div > button".into(), "layout".into())),
            "{kinds:?}"
        );
        assert!(
            kinds.contains(&("div > em".into(), "added".into())),
            "{kinds:?}"
        );
        assert!(
            kinds.contains(&("div > span".into(), "removed".into())),
            "{kinds:?}"
        );
        let style = changes.iter().find(|c| c.kind == "style").unwrap();
        assert_eq!(style.property.as_deref(), Some("font-weight"));
        assert_eq!(style.before, json!("500"));
        assert_eq!(style.after, json!("600"));
        assert_eq!(style.component.as_deref(), Some("Button"));
        assert!(diff(&a, &a).is_empty());
    }

    #[test]
    fn regions_map_to_the_deepest_elements() {
        let dom = vec![node(
            "div",
            None,
            json!({}),
            vec![
                node(
                    "button",
                    Some("Save"),
                    json!({}),
                    vec![node(
                        "span",
                        Some("S"),
                        json!({}),
                        vec![],
                        (4.0, 4.0, 20.0, 16.0),
                    )],
                    (0.0, 0.0, 60.0, 24.0),
                ),
                node(
                    "span",
                    Some("hint"),
                    json!({}),
                    vec![],
                    (70.0, 0.0, 30.0, 24.0),
                ),
            ],
            (0.0, 0.0, 100.0, 24.0),
        )];
        let region = Region {
            x: 6,
            y: 6,
            w: 10,
            h: 10,
            changed: 100,
            ratio: 1.0,
        };
        let paths = elements_for_region(&dom, &region);
        assert_eq!(paths[0], "div > button > span");
        assert!(!paths.iter().any(|p| p == "div"), "{paths:?}");
        let far = Region {
            x: 72,
            y: 2,
            w: 4,
            h: 4,
            changed: 16,
            ratio: 1.0,
        };
        assert_eq!(
            elements_for_region(&dom, &far),
            vec!["div > span".to_string()]
        );
    }
}
