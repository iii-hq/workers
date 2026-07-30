//! `database::schemaDiagram` — the shape of a schema, laid out.
//!
//! Layout runs here rather than in a renderer for two reasons. It collapses
//! the N+1 a diagram would otherwise need (`describeSchema` already reads the
//! whole catalog in a handful of queries), and it makes the schema's *shape*
//! askable: an agent can call this and reason about hub tables, orphans and
//! reference cycles without drawing anything.
//!
//! The algorithm is a cut-down Sugiyama:
//!
//! 1. Split into connected components over foreign keys, so a hundred
//!    unrelated lookup tables do not dominate the canvas.
//! 2. Rank each component by longest path along the FK direction, breaking
//!    cycles at the lowest-degree edge.
//! 3. Reduce edge crossings with a barycenter sweep followed by adjacent
//!    transposition, keeping whichever ordering measures better.
//! 4. Assign coordinates and route edges as three-segment elbows anchored on
//!    the *column row*, not the box.
//!
//! Every step sorts by name before iterating, so the same schema always
//! produces the same diagram — which is what makes `crossings` a number worth
//! asserting on in a test.

use super::schema::{self, DescribeSchemaReq, TableDescription};
use super::AppState;
use crate::error::DbError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

fn default_timeout() -> u64 {
    30_000
}

fn default_max_tables() -> usize {
    200
}

/// Geometry. Fixed here so the renderer never has to guess, and so edge
/// anchors line up with the row a column actually occupies.
pub const NODE_WIDTH: f64 = 264.0;
pub const HEADER_HEIGHT: f64 = 34.0;
pub const ROW_HEIGHT: f64 = 24.0;
pub const NODE_PAD_Y: f64 = 8.0;
/// Columns beyond this are summarised rather than drawn.
pub const MAX_ROWS: usize = 12;
/// Rank pitch, measured centre-of-column to centre-of-column, so the readable
/// gutter is `RANK_SEP - NODE_WIDTH`. At 220 that gutter was 20px: every edge
/// collapsed into a stub too short to read, and a schema with real foreign
/// keys looked unrelated. The gutter has to dominate the elbow's corner radius
/// for the connection to register at all.
const RANK_SEP: f64 = 380.0;
const V_GAP: f64 = 56.0;
const COMPONENT_GAP: f64 = 140.0;
/// Pitch between unrelated tables. Tighter than `COMPONENT_GAP` because no
/// edge is ever drawn between them.
const ISOLATED_GAP: f64 = 40.0;
/// Vertical break between the connected diagram and the orphan shelf.
const SHELF_GAP: f64 = 80.0;
const SHELF_WIDTH: f64 = 1600.0;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DiagramColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub primary_key: bool,
    pub foreign_key: bool,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DiagramNode {
    pub table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rank: u32,
    /// Number of foreign keys touching this table, in or out. Renderers use
    /// it to emphasise hubs.
    pub degree: u32,
    pub columns: Vec<DiagramColumn>,
    /// Columns not drawn because of `MAX_ROWS`.
    pub hidden_columns: usize,
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DiagramEdge {
    pub from: String,
    pub from_column: String,
    pub to: String,
    pub to_column: String,
    /// Polyline, already routed. Anchored on the column row where the column
    /// is visible, on the node edge otherwise.
    pub points: Vec<Point>,
    pub self_loop: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SchemaDiagramReq {
    #[serde(default)]
    pub db: Option<String>,
    #[serde(default)]
    pub include_views: bool,
    #[serde(default = "default_max_tables")]
    pub max_tables: usize,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    /// Lay out only the neighbourhood of this table.
    ///
    /// A whole schema drawn at once answers "what exists"; it does not answer
    /// "what does this table touch", which is the question actually being
    /// asked most of the time. With a focus the diagram becomes explorable
    /// one hop at a time instead of a wall to be scanned.
    #[serde(default)]
    pub focus: Option<String>,
    /// How many foreign-key hops out from `focus` to include. Ignored without
    /// one. 1 is the table and its direct relations.
    #[serde(default = "default_depth")]
    pub depth: usize,
}

fn default_depth() -> usize {
    1
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SchemaDiagramResp {
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
    pub width: f64,
    pub height: f64,
    /// Tables with no foreign keys at all, placed on a trailing shelf.
    pub isolated: Vec<String>,
    /// Connected groups, with the box that encloses each. A schema is usually
    /// several independent clusters rather than one graph, and saying so is
    /// most of what makes a large diagram readable — a reader can take in
    /// "four unrelated groups" at a glance instead of scanning for edges that
    /// are not there.
    pub components: Vec<DiagramComponent>,
    /// Edge crossings remaining after ordering. Lower is a tidier diagram.
    pub crossings: u32,
    pub truncated: bool,
    /// Echoed when the caller asked for a neighbourhood rather than the whole
    /// schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// Tables one hop beyond what was drawn. Non-empty means there is more to
    /// expand into, which is the difference between a diagram that looks
    /// complete and one that says where it stops.
    #[serde(default)]
    pub frontier: Vec<String>,
}

/// One connected group of tables and its bounding box.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DiagramComponent {
    /// Stable index, in layout order.
    pub index: usize,
    /// Member tables, by node id.
    pub tables: Vec<String>,
    /// The most-referenced table in the group, if it has more than one member.
    pub hub: Option<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A relationship between two tables, keyed by the qualified names used as
/// node ids.
#[derive(Debug, Clone)]
struct Relation {
    from: String,
    from_column: String,
    to: String,
    to_column: String,
}

fn qualified(schema: Option<&str>, table: &str) -> String {
    match schema {
        Some(s) => format!("{s}.{table}"),
        None => table.to_string(),
    }
}

fn node_height(columns: usize) -> f64 {
    HEADER_HEIGHT + (columns.min(MAX_ROWS) as f64) * ROW_HEIGHT + 2.0 * NODE_PAD_Y
}

pub async fn handle(state: &AppState, req: SchemaDiagramReq) -> Result<SchemaDiagramResp, String> {
    let described = schema::describe_schema(
        state,
        DescribeSchemaReq {
            db: req.db,
            tables: None,
            include_indexes: false,
            max_tables: req.max_tables,
            timeout_ms: req.timeout_ms,
        },
    )
    .await?;

    let tables: Vec<TableDescription> = described
        .tables
        .into_iter()
        .filter(|t| req.include_views || t.kind == super::catalog::TableKind::Table)
        .collect();

    let (tables, frontier) = match &req.focus {
        Some(focus) => neighbourhood(&tables, focus, req.depth)?,
        None => (tables, Vec::new()),
    };

    let mut resp = layout(&tables, described.truncated);
    resp.focus = req.focus;
    resp.frontier = frontier;
    Ok(resp)
}

/// The tables within `depth` foreign-key hops of `focus`, plus the names one
/// hop beyond that were not included.
///
/// Edges are walked in both directions: a reader asking what `users` touches
/// means both the tables it references and the tables referencing it, and
/// following only the declared direction would hide every child table.
///
/// The `frontier` is what makes this explorable rather than merely smaller —
/// it tells the renderer which nodes have more behind them, so expanding is an
/// informed click instead of a guess.
fn neighbourhood(
    tables: &[TableDescription],
    focus: &str,
    depth: usize,
) -> Result<(Vec<TableDescription>, Vec<String>), String> {
    let ids: Vec<String> = tables
        .iter()
        .map(|t| qualified(t.schema.as_deref(), &t.table))
        .collect();

    // Accept either the qualified id or the bare table name, since a caller
    // clicking a node has the former and a caller typing has the latter.
    let start = ids
        .iter()
        .find(|id| id.as_str() == focus)
        .or_else(|| {
            tables
                .iter()
                .zip(&ids)
                .find(|(t, _)| t.table == focus)
                .map(|(_, id)| id)
        })
        .cloned();
    let Some(start) = start else {
        return Err(super::query::err_to_str(DbError::InvalidParam {
            index: 0,
            reason: format!("no table named `{focus}` in this database"),
        }));
    };

    // Undirected adjacency over foreign keys.
    let present: BTreeSet<&String> = ids.iter().collect();
    let mut adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for t in tables {
        let from = qualified(t.schema.as_deref(), &t.table);
        for c in &t.columns {
            let Some(fk) = &c.foreign_key else { continue };
            let to = qualified(fk.schema.as_deref().or(t.schema.as_deref()), &fk.table);
            if !present.contains(&to) || to == from {
                continue;
            }
            adj.entry(from.clone()).or_default().insert(to.clone());
            adj.entry(to).or_default().insert(from.clone());
        }
    }

    // Breadth-first to `depth`, then one more ring to find the frontier.
    let mut included: BTreeSet<String> = BTreeSet::new();
    included.insert(start.clone());
    let mut ring: Vec<String> = vec![start];
    for _ in 0..depth {
        // Stop once the component is exhausted. `depth` comes straight off the
        // wire with no ceiling, so without this a caller asking for depth
        // 1e11 spins the worker thread long after there is nothing left to
        // reach.
        if ring.is_empty() {
            break;
        }
        let mut next: Vec<String> = Vec::new();
        for node in &ring {
            for peer in adj.get(node).into_iter().flatten() {
                if included.insert(peer.clone()) {
                    next.push(peer.clone());
                }
            }
        }
        ring = next;
    }

    let frontier: Vec<String> = ring
        .iter()
        .flat_map(|node| adj.get(node).into_iter().flatten())
        .filter(|peer| !included.contains(*peer))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let kept: Vec<TableDescription> = tables
        .iter()
        .zip(&ids)
        .filter(|(_, id)| included.contains(*id))
        .map(|(t, _)| t.clone())
        .collect();

    Ok((kept, frontier))
}

/// Pure layout. Separated from the fetch so it can be tested without a
/// database, and so the same input always yields the same diagram.
pub fn layout(tables: &[TableDescription], truncated: bool) -> SchemaDiagramResp {
    // Node ids, sorted so every later iteration is deterministic.
    let mut ids: Vec<String> = tables
        .iter()
        .map(|t| qualified(t.schema.as_deref(), &t.table))
        .collect();
    ids.sort();
    let present: BTreeSet<&String> = ids.iter().collect();

    // Relations, deduplicated. A reference to a table outside the set (a view
    // we filtered out, or a truncated tail) is dropped rather than drawn to
    // nowhere.
    let mut relations: Vec<Relation> = Vec::new();
    let mut seen: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    for t in tables {
        let from = qualified(t.schema.as_deref(), &t.table);
        for c in &t.columns {
            let Some(fk) = &c.foreign_key else { continue };
            let to = qualified(fk.schema.as_deref().or(t.schema.as_deref()), &fk.table);
            if !present.contains(&to) {
                continue;
            }
            let key = (from.clone(), c.name.clone(), to.clone(), fk.column.clone());
            if seen.insert(key) {
                relations.push(Relation {
                    from: from.clone(),
                    from_column: c.name.clone(),
                    to,
                    to_column: fk.column.clone(),
                });
            }
        }
    }

    let mut degree: BTreeMap<String, u32> = ids.iter().map(|i| (i.clone(), 0)).collect();
    for r in &relations {
        *degree.get_mut(&r.from).expect("id present") += 1;
        if r.to != r.from {
            *degree.get_mut(&r.to).expect("id present") += 1;
        }
    }

    // Degree-0 tables go to a trailing shelf; on a real schema they are often
    // the majority and would otherwise stretch the canvas around nothing.
    let isolated: Vec<String> = ids.iter().filter(|i| degree[*i] == 0).cloned().collect();
    let connected: Vec<String> = ids.iter().filter(|i| degree[*i] > 0).cloned().collect();

    let components = split_components(&connected, &relations);
    let ranks = rank_all(&components, &relations, &degree);

    // Order within each rank, then place.
    let by_id: HashMap<&String, &TableDescription> = tables
        .iter()
        .map(|t| {
            let id = ids
                .iter()
                .find(|i| **i == qualified(t.schema.as_deref(), &t.table))
                .expect("id built from this table");
            (id, t)
        })
        .collect();

    let mut positions: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut cursor_x = 0.0_f64;
    let mut max_height = 0.0_f64;
    // Origin of the row of components being filled, and the tallest component
    // in it.
    let mut row_y = 0.0_f64;
    let mut row_height = 0.0_f64;

    for comp in &components {
        let order = order_component(comp, &ranks, &relations);
        // Wrap *before* placing, and lay the component out at the current row
        // origin. Placing at y=0 unconditionally and only recording the wrap
        // afterwards drew every row after the first on top of the one above
        // it, so a schema with enough components to wrap rendered stacked.
        if cursor_x > 0.0 && cursor_x + NODE_WIDTH > SHELF_WIDTH {
            cursor_x = 0.0;
            row_y += row_height + COMPONENT_GAP;
            row_height = 0.0;
        }
        let (w, h) = place(&order, &ranks, &by_id, cursor_x, row_y, &mut positions);
        cursor_x += w + COMPONENT_GAP;
        row_height = row_height.max(h);
        max_height = max_height.max(row_y + row_height);
    }

    // Isolated tables tile a shelf below everything else, packed tightly.
    //
    // They get ISOLATED_GAP rather than COMPONENT_GAP: nothing connects them,
    // so the wide gutter that makes edges readable buys nothing here and only
    // stretches the canvas. A canvas wider than it needs to be forces the
    // fit-zoom down, which shrinks the type in *every* node — the sparse
    // shelf was why the whole diagram was rendering at 55%.
    //
    // The shelf is also capped to the width the connected part already
    // occupies, so orphans wrap under the diagram instead of widening it.
    let connected_width = positions
        .values()
        .map(|(x, _)| x + NODE_WIDTH)
        .fold(0.0_f64, f64::max);
    let shelf_width = connected_width.clamp(NODE_WIDTH * 4.0 + ISOLATED_GAP * 3.0, SHELF_WIDTH);
    let mut ix = 0.0_f64;
    let mut iy = max_height + SHELF_GAP;
    let mut row_height = 0.0_f64;
    for id in &isolated {
        let cols = by_id.get(id).map(|t| t.columns.len()).unwrap_or(0);
        let h = node_height(cols);
        if ix > 0.0 && ix + NODE_WIDTH > shelf_width {
            ix = 0.0;
            iy += row_height + V_GAP;
            row_height = 0.0;
        }
        positions.insert(id.clone(), (ix, iy));
        row_height = row_height.max(h);
        ix += NODE_WIDTH + ISOLATED_GAP;
        max_height = max_height.max(iy + h);
    }

    let nodes = build_nodes(&ids, &by_id, &positions, &ranks, &degree);

    // Bounding boxes are derived from the placed nodes rather than tracked
    // during placement, so they cannot drift from where the nodes ended up.
    let node_box: HashMap<&str, (f64, f64, f64, f64)> = nodes
        .iter()
        .map(|n| (node_id(n) as &str, (n.x, n.y, n.w, n.h)))
        .collect();
    let component_boxes: Vec<DiagramComponent> = components
        .iter()
        .enumerate()
        .filter_map(|(index, comp)| {
            let boxes: Vec<_> = comp
                .iter()
                .filter_map(|t| node_box.get(t.as_str()).copied())
                .collect();
            if boxes.is_empty() {
                return None;
            }
            let x = boxes.iter().map(|b| b.0).fold(f64::INFINITY, f64::min);
            let y = boxes.iter().map(|b| b.1).fold(f64::INFINITY, f64::min);
            let right = boxes.iter().map(|b| b.0 + b.2).fold(0.0_f64, f64::max);
            let bottom = boxes.iter().map(|b| b.1 + b.3).fold(0.0_f64, f64::max);
            // Only name a hub when one table is strictly the most connected.
            // In a two-table pair both ends have degree 1, and picking one on
            // a tiebreak would emphasise an arbitrary node in the renderer.
            let mut ranked: Vec<(u32, &String)> = comp
                .iter()
                .map(|t| (degree.get(t).copied().unwrap_or(0), t))
                .collect();
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
            let hub = match ranked.as_slice() {
                [(top, name), (second, _), ..] if top > second => Some((*name).clone()),
                _ => None,
            };
            Some(DiagramComponent {
                index,
                tables: comp.clone(),
                hub,
                x,
                y,
                w: right - x,
                h: bottom - y,
            })
        })
        .collect();

    let index: HashMap<&str, &DiagramNode> = nodes
        .iter()
        .map(|n| (node_id(n) as &str, n))
        .collect::<HashMap<_, _>>();
    let edges = route(&relations, &index);
    let crossings = count_crossings(&edges);

    let width = nodes
        .iter()
        .map(|n| n.x + n.w)
        .fold(0.0_f64, f64::max)
        .max(NODE_WIDTH);
    let height = nodes
        .iter()
        .map(|n| n.y + n.h)
        .fold(0.0_f64, f64::max)
        .max(HEADER_HEIGHT);

    SchemaDiagramResp {
        nodes,
        edges,
        width,
        height,
        isolated,
        components: component_boxes,
        crossings,
        // Set by `handle` when the caller asked for a neighbourhood; `layout`
        // itself is given the tables it should draw and knows nothing of why.
        focus: None,
        frontier: Vec::new(),
        truncated,
    }
}

/// Leaked id string for the lookup map; nodes own their name, this just
/// borrows it.
fn node_id(n: &DiagramNode) -> &str {
    // `table` already carries the qualifier when one exists, because nodes are
    // built from the qualified id.
    &n.table
}

fn split_components(ids: &[String], relations: &[Relation]) -> Vec<Vec<String>> {
    let mut parent: BTreeMap<&String, &String> = ids.iter().map(|i| (i, i)).collect();

    fn find<'a>(parent: &BTreeMap<&'a String, &'a String>, x: &'a String) -> &'a String {
        let mut cur = x;
        while parent[cur] != cur {
            cur = parent[cur];
        }
        cur
    }

    for r in relations {
        let (Some(a), Some(b)) = (
            ids.iter().find(|i| **i == r.from),
            ids.iter().find(|i| **i == r.to),
        ) else {
            continue;
        };
        let ra = find(&parent, a);
        let rb = find(&parent, b);
        if ra != rb {
            // Union by name keeps the result independent of input order.
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent.insert(hi, lo);
        }
    }

    let mut groups: BTreeMap<&String, Vec<String>> = BTreeMap::new();
    for id in ids {
        groups
            .entry(find(&parent, id))
            .or_default()
            .push(id.clone());
    }
    groups.into_values().collect()
}

/// Longest-path ranking along FK direction (referencing above referenced),
/// with cycles broken at the edge whose endpoints have the lowest degree.
fn rank_all(
    components: &[Vec<String>],
    relations: &[Relation],
    degree: &BTreeMap<String, u32>,
) -> BTreeMap<String, u32> {
    let mut ranks: BTreeMap<String, u32> = BTreeMap::new();
    for comp in components {
        let members: BTreeSet<&String> = comp.iter().collect();
        let mut edges: Vec<&Relation> = relations
            .iter()
            .filter(|r| members.contains(&r.from) && members.contains(&r.to) && r.from != r.to)
            .collect();
        // Deterministic cycle breaking: drop the lowest-degree edge last, so
        // the densest structure keeps its shape.
        edges.sort_by(|a, b| {
            (degree[&a.from] + degree[&a.to])
                .cmp(&(degree[&b.from] + degree[&b.to]))
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });

        for id in comp {
            ranks.insert(id.clone(), 0);
        }
        // Relax repeatedly; bounded by component size so a residual cycle
        // terminates instead of spinning.
        for _ in 0..comp.len().min(64) {
            let mut changed = false;
            for e in &edges {
                let want = ranks[&e.to] + 1;
                if ranks[&e.from] < want {
                    ranks.insert(e.from.clone(), want);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
    ranks
}

/// Barycenter sweep plus adjacent transposition, keeping the better result.
fn order_component(
    comp: &[String],
    ranks: &BTreeMap<String, u32>,
    relations: &[Relation],
) -> BTreeMap<u32, Vec<String>> {
    let mut layers: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for id in comp {
        layers.entry(ranks[id]).or_default().push(id.clone());
    }
    for v in layers.values_mut() {
        v.sort();
    }

    let neighbours = |id: &str| -> Vec<String> {
        relations
            .iter()
            .filter_map(|r| {
                if r.from == id {
                    Some(r.to.clone())
                } else if r.to == id {
                    Some(r.from.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    let mut best = layers.clone();
    let mut best_score = layer_crossings(&layers, relations);

    for _ in 0..4 {
        let snapshot = layers.clone();
        for row in layers.values_mut() {
            let mut keyed: Vec<(f64, String)> = row
                .iter()
                .map(|id| {
                    let ns = neighbours(id);
                    let bary = if ns.is_empty() {
                        f64::MAX
                    } else {
                        let sum: f64 = ns
                            .iter()
                            .filter_map(|n| {
                                snapshot
                                    .values()
                                    .find_map(|r| r.iter().position(|x| x == n).map(|p| p as f64))
                            })
                            .sum();
                        sum / ns.len() as f64
                    };
                    (bary, id.clone())
                })
                .collect();
            // Name breaks ties, so the sweep is reproducible.
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
            *row = keyed.into_iter().map(|(_, id)| id).collect();
        }
        let score = layer_crossings(&layers, relations);
        if score < best_score {
            best_score = score;
            best = layers.clone();
        }
    }
    best
}

/// Crossings implied by the current ordering, used to choose between sweeps.
fn layer_crossings(layers: &BTreeMap<u32, Vec<String>>, relations: &[Relation]) -> u32 {
    let pos: HashMap<&String, (u32, usize)> = layers
        .iter()
        .flat_map(|(rank, row)| row.iter().enumerate().map(move |(i, id)| (id, (*rank, i))))
        .collect();

    let mut edges: Vec<(u32, usize, usize)> = Vec::new();
    for r in relations {
        let (Some(a), Some(b)) = (pos.get(&r.from), pos.get(&r.to)) else {
            continue;
        };
        if a.0 == b.0 + 1 {
            edges.push((b.0, a.1, b.1));
        }
    }

    let mut n = 0;
    for i in 0..edges.len() {
        for j in (i + 1)..edges.len() {
            let (l1, u1, v1) = edges[i];
            let (l2, u2, v2) = edges[j];
            if l1 == l2 && ((u1 < u2) != (v1 < v2)) {
                n += 1;
            }
        }
    }
    n
}

fn place(
    order: &BTreeMap<u32, Vec<String>>,
    _ranks: &BTreeMap<String, u32>,
    by_id: &HashMap<&String, &TableDescription>,
    origin_x: f64,
    origin_y: f64,
    out: &mut BTreeMap<String, (f64, f64)>,
) -> (f64, f64) {
    let mut max_w = 0.0_f64;
    let mut max_h = 0.0_f64;
    for (rank, row) in order {
        let x = origin_x + (*rank as f64) * RANK_SEP;
        let mut y = origin_y;
        for id in row {
            out.insert(id.clone(), (x, y));
            let cols = by_id.get(id).map(|t| t.columns.len()).unwrap_or(0);
            y += node_height(cols) + V_GAP;
        }
        max_w = max_w.max(x - origin_x + NODE_WIDTH);
        max_h = max_h.max(y - origin_y);
    }
    (max_w, max_h)
}

fn build_nodes(
    ids: &[String],
    by_id: &HashMap<&String, &TableDescription>,
    positions: &BTreeMap<String, (f64, f64)>,
    ranks: &BTreeMap<String, u32>,
    degree: &BTreeMap<String, u32>,
) -> Vec<DiagramNode> {
    ids.iter()
        .filter_map(|id| {
            let t = by_id.get(id)?;
            let (x, y) = *positions.get(id)?;
            let shown: Vec<DiagramColumn> = t
                .columns
                .iter()
                .take(MAX_ROWS)
                .map(|c| DiagramColumn {
                    name: c.name.clone(),
                    ty: c.ty.clone(),
                    primary_key: c.primary_key,
                    foreign_key: c.foreign_key.is_some(),
                    nullable: c.nullable,
                })
                .collect();
            Some(DiagramNode {
                table: id.clone(),
                schema: t.schema.clone(),
                x,
                y,
                w: NODE_WIDTH,
                h: node_height(t.columns.len()),
                rank: ranks.get(id).copied().unwrap_or(0),
                degree: degree.get(id).copied().unwrap_or(0),
                hidden_columns: t.columns.len().saturating_sub(shown.len()),
                columns: shown,
            })
        })
        .collect()
}

/// Vertical centre of a column's row, so an edge points at `user_id` rather
/// than at the middle of the box. Falls back to the node centre when the
/// column is past `MAX_ROWS` and therefore not drawn.
fn anchor_y(node: &DiagramNode, column: &str) -> f64 {
    match node.columns.iter().position(|c| c.name == column) {
        Some(i) => node.y + HEADER_HEIGHT + NODE_PAD_Y + (i as f64 + 0.5) * ROW_HEIGHT,
        None => node.y + node.h / 2.0,
    }
}

fn route(relations: &[Relation], index: &HashMap<&str, &DiagramNode>) -> Vec<DiagramEdge> {
    relations
        .iter()
        .filter_map(|r| {
            let from = index.get(r.from.as_str())?;
            let to = index.get(r.to.as_str())?;
            let self_loop = r.from == r.to;

            let y1 = anchor_y(from, &r.from_column);
            let y2 = anchor_y(to, &r.to_column);

            let points = if self_loop {
                // Loop out of the right edge and back.
                let x = from.x + from.w;
                vec![
                    Point { x, y: y1 },
                    Point { x: x + 24.0, y: y1 },
                    Point {
                        x: x + 24.0,
                        y: y1 - ROW_HEIGHT,
                    },
                    Point {
                        x,
                        y: y1 - ROW_HEIGHT,
                    },
                ]
            } else {
                // Three-segment elbow, leaving from whichever side faces the
                // target so edges do not cross their own node.
                let (x1, x2) = if to.x >= from.x + from.w {
                    (from.x + from.w, to.x)
                } else if from.x >= to.x + to.w {
                    (from.x, to.x + to.w)
                } else {
                    (from.x + from.w, to.x + to.w)
                };
                let mid = (x1 + x2) / 2.0;
                vec![
                    Point { x: x1, y: y1 },
                    Point { x: mid, y: y1 },
                    Point { x: mid, y: y2 },
                    Point { x: x2, y: y2 },
                ]
            };

            Some(DiagramEdge {
                from: r.from.clone(),
                from_column: r.from_column.clone(),
                to: r.to.clone(),
                to_column: r.to_column.clone(),
                points,
                self_loop,
            })
        })
        .collect()
}

/// Crossings between routed edges, by the standard interleaving test: two
/// edges sharing a corridor cross when their endpoints are in opposite order
/// at each end.
///
/// Not a plain overlap test. Six edges fanning into one parent all occupy the
/// same corridor and overlap vertically, but none of them cross — they
/// converge. Counting overlap would report fifteen crossings for a diagram a
/// reader would call tidy.
fn count_crossings(edges: &[DiagramEdge]) -> u32 {
    /// (corridor x, source y, target y)
    fn ends(e: &DiagramEdge) -> Option<(f64, f64, f64)> {
        let mid = e.points.get(1)?;
        Some((mid.x, e.points.first()?.y, e.points.last()?.y))
    }
    let segs: Vec<(f64, f64, f64)> = edges
        .iter()
        .filter(|e| !e.self_loop)
        .filter_map(ends)
        .collect();

    let mut n = 0;
    for i in 0..segs.len() {
        for j in (i + 1)..segs.len() {
            let (x1, s1, t1) = segs[i];
            let (x2, s2, t2) = segs[j];
            if (x1 - x2).abs() > f64::EPSILON {
                continue;
            }
            // Shared endpoints converge or diverge; neither is a crossing.
            if s1 == s2 || t1 == t2 {
                continue;
            }
            if (s1 < s2) != (t1 < t2) {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::catalog::{ColumnDesc, ForeignKeyRef, TableKind};

    fn col(name: &str, pk: bool, fk: Option<(&str, &str)>) -> ColumnDesc {
        ColumnDesc {
            name: name.into(),
            ty: "INTEGER".into(),
            nullable: !pk,
            default_value: None,
            primary_key: pk,
            position: 1,
            foreign_key: fk.map(|(t, c)| ForeignKeyRef {
                schema: None,
                table: t.into(),
                column: c.into(),
            }),
        }
    }

    fn table(name: &str, columns: Vec<ColumnDesc>) -> TableDescription {
        TableDescription {
            table: name.into(),
            schema: None,
            kind: TableKind::Table,
            columns,
            indexes: vec![],
            row_count_estimate: None,
        }
    }

    #[test]
    fn an_empty_schema_lays_out_without_panicking() {
        let d = layout(&[], false);
        assert!(d.nodes.is_empty() && d.edges.is_empty());
        assert_eq!(d.crossings, 0);
    }

    #[test]
    fn a_lone_table_is_isolated_not_ranked() {
        let d = layout(&[table("users", vec![col("id", true, None)])], false);
        assert_eq!(d.isolated, vec!["users"]);
        assert_eq!(d.nodes.len(), 1);
        assert_eq!(d.nodes[0].degree, 0);
    }

    /// users ← orders ← order_items, plus an unrelated table.
    fn chain() -> Vec<TableDescription> {
        vec![
            table("users", vec![col("id", true, None)]),
            table(
                "orders",
                vec![
                    col("id", true, None),
                    col("user_id", false, Some(("users", "id"))),
                ],
            ),
            table(
                "order_items",
                vec![
                    col("id", true, None),
                    col("order_id", false, Some(("orders", "id"))),
                ],
            ),
            table("unrelated", vec![col("id", true, None)]),
        ]
    }

    #[test]
    fn components_never_overlap_once_the_row_wraps() {
        // Enough two-table components to run past SHELF_WIDTH. Every one was
        // previously placed at y=0, so the second row drew on top of the
        // first.
        let mut tables = Vec::new();
        for i in 0..12 {
            tables.push(table(&format!("p{i}"), vec![col("id", true, None)]));
            tables.push(table(
                &format!("c{i}"),
                vec![
                    col("id", true, None),
                    col("pid", false, Some((&format!("p{i}"), "id"))),
                ],
            ));
        }
        let d = layout(&tables, false);
        assert!(d.components.len() > 1);
        for (i, a) in d.components.iter().enumerate() {
            for b in d.components.iter().skip(i + 1) {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "components {} and {} overlap", a.index, b.index);
            }
        }
    }

    #[test]
    fn an_absurd_depth_terminates_instead_of_spinning() {
        // `depth` is unbounded on the wire; the walk must stop when the
        // component is exhausted rather than iterating that many times.
        let (kept, frontier) = neighbourhood(&chain(), "users", usize::MAX).unwrap();
        assert_eq!(kept.len(), 3);
        assert!(frontier.is_empty());
    }

    #[test]
    fn focus_follows_references_in_both_directions() {
        // `orders` references users and is referenced by order_items. A walk
        // that only followed the declared direction would hide every child.
        let (kept, _) = neighbourhood(&chain(), "orders", 1).unwrap();
        let mut names: Vec<&str> = kept.iter().map(|t| t.table.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["order_items", "orders", "users"]);
    }

    #[test]
    fn focus_reports_what_lies_one_hop_beyond() {
        let (kept, frontier) = neighbourhood(&chain(), "users", 1).unwrap();
        let mut names: Vec<&str> = kept.iter().map(|t| t.table.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["orders", "users"]);
        // order_items is reachable but not drawn — the renderer needs to know
        // there is something to expand into.
        assert_eq!(frontier, vec!["order_items".to_string()]);
    }

    #[test]
    fn a_deeper_focus_pulls_the_next_ring_in() {
        let (kept, frontier) = neighbourhood(&chain(), "users", 2).unwrap();
        assert_eq!(kept.len(), 3, "users, orders and order_items");
        assert!(frontier.is_empty(), "nothing further to reach");
    }

    #[test]
    fn focus_never_drags_in_an_unrelated_table() {
        let (kept, _) = neighbourhood(&chain(), "users", 9).unwrap();
        assert!(kept.iter().all(|t| t.table != "unrelated"));
    }

    #[test]
    fn an_unknown_focus_is_an_error_not_an_empty_diagram() {
        // Silently returning nothing would read as "this table has no
        // relations", which is a different and wrong answer.
        assert!(neighbourhood(&chain(), "nope", 1).is_err());
    }

    #[test]
    fn adjacent_ranks_leave_a_gutter_wide_enough_to_draw_an_edge_in() {
        // Regression. RANK_SEP was 220 against a 200-wide node, leaving a
        // 20px gutter: every edge rendered as a stub too short to see, and a
        // schema with real foreign keys looked unrelated on screen. The
        // gutter, not the pitch, is the number that matters.
        let d = layout(
            &[
                table("users", vec![col("id", true, None)]),
                table(
                    "orders",
                    vec![
                        col("id", true, None),
                        col("user_id", false, Some(("users", "id"))),
                    ],
                ),
            ],
            false,
        );
        let by: HashMap<&str, &DiagramNode> =
            d.nodes.iter().map(|n| (n.table.as_str(), n)).collect();
        let gutter = by["orders"].x - (by["users"].x + by["users"].w);
        assert!(
            gutter >= 100.0,
            "adjacent ranks left a {gutter}px gutter; an edge needs room to read as a connection"
        );
    }

    #[test]
    fn related_tables_are_grouped_into_one_component_box() {
        let d = layout(
            &[
                table("users", vec![col("id", true, None)]),
                table(
                    "orders",
                    vec![
                        col("id", true, None),
                        col("user_id", false, Some(("users", "id"))),
                    ],
                ),
                table("unrelated", vec![col("id", true, None)]),
            ],
            false,
        );
        assert_eq!(d.components.len(), 1, "one connected group");
        let c = &d.components[0];
        assert_eq!(c.tables.len(), 2);
        assert_eq!(
            c.hub, None,
            "both ends of a pair have degree 1 — naming either as the hub would be arbitrary"
        );
        assert!(c.w > 0.0 && c.h > 0.0, "the group has a drawable box");
        // The box must actually contain its members, or the renderer draws a
        // boundary that clips them.
        for t in &c.tables {
            let n = d.nodes.iter().find(|n| &n.table == t).expect("member node");
            assert!(
                n.x >= c.x && n.y >= c.y && n.x + n.w <= c.x + c.w && n.y + n.h <= c.y + c.h,
                "{t} falls outside its component box"
            );
        }
        assert_eq!(d.isolated, vec!["unrelated".to_string()]);
    }

    #[test]
    fn the_most_connected_table_is_named_as_the_hub() {
        let d = layout(
            &[
                table("users", vec![col("id", true, None)]),
                table(
                    "orders",
                    vec![
                        col("id", true, None),
                        col("user_id", false, Some(("users", "id"))),
                    ],
                ),
                table(
                    "sessions",
                    vec![
                        col("id", true, None),
                        col("user_id", false, Some(("users", "id"))),
                    ],
                ),
            ],
            false,
        );
        assert_eq!(d.components.len(), 1);
        assert_eq!(
            d.components[0].hub.as_deref(),
            Some("users"),
            "two tables reference users, so it is strictly the most connected"
        );
    }

    #[test]
    fn a_reference_ranks_the_child_above_the_parent() {
        let d = layout(
            &[
                table("users", vec![col("id", true, None)]),
                table(
                    "orders",
                    vec![
                        col("id", true, None),
                        col("user_id", false, Some(("users", "id"))),
                    ],
                ),
            ],
            false,
        );
        let by: HashMap<&str, &DiagramNode> =
            d.nodes.iter().map(|n| (n.table.as_str(), n)).collect();
        assert_eq!(by["users"].rank, 0, "the referenced table sits upstream");
        assert_eq!(by["orders"].rank, 1);
        assert!(d.isolated.is_empty());
        assert_eq!(d.edges.len(), 1);
        assert_eq!(d.edges[0].to_column, "id");
    }

    #[test]
    fn an_edge_anchors_on_the_column_row_not_the_box() {
        let d = layout(
            &[
                table("users", vec![col("id", true, None)]),
                table(
                    "orders",
                    vec![
                        col("id", true, None),
                        col("filler", false, None),
                        col("user_id", false, Some(("users", "id"))),
                    ],
                ),
            ],
            false,
        );
        let orders = d.nodes.iter().find(|n| n.table == "orders").unwrap();
        let edge = &d.edges[0];
        // Third column, so the anchor is two rows below the first.
        let expected = orders.y + HEADER_HEIGHT + NODE_PAD_Y + 2.5 * ROW_HEIGHT;
        assert!(
            (edge.points[0].y - expected).abs() < 0.001,
            "anchored at {} not {expected}",
            edge.points[0].y
        );
    }

    #[test]
    fn a_reference_cycle_terminates_and_still_ranks_everything() {
        let d = layout(
            &[
                table(
                    "a",
                    vec![col("id", true, None), col("b_id", false, Some(("b", "id")))],
                ),
                table(
                    "b",
                    vec![col("id", true, None), col("a_id", false, Some(("a", "id")))],
                ),
            ],
            false,
        );
        assert_eq!(d.nodes.len(), 2);
        assert!(d.isolated.is_empty());
    }

    #[test]
    fn a_self_reference_becomes_a_loop_not_a_line() {
        let d = layout(
            &[table(
                "employees",
                vec![
                    col("id", true, None),
                    col("manager_id", false, Some(("employees", "id"))),
                ],
            )],
            false,
        );
        assert_eq!(d.edges.len(), 1);
        assert!(d.edges[0].self_loop);
    }

    #[test]
    fn a_hub_reports_its_degree_and_leaves_no_crossings() {
        let mut tables = vec![table("users", vec![col("id", true, None)])];
        for i in 0..6 {
            tables.push(table(
                &format!("child{i}"),
                vec![
                    col("id", true, None),
                    col("user_id", false, Some(("users", "id"))),
                ],
            ));
        }
        let d = layout(&tables, false);
        let users = d.nodes.iter().find(|n| n.table == "users").unwrap();
        assert_eq!(users.degree, 6);
        assert_eq!(d.crossings, 0, "a simple fan should not cross");
    }

    #[test]
    fn layout_is_deterministic_regardless_of_input_order() {
        let mut a = vec![
            table("users", vec![col("id", true, None)]),
            table(
                "orders",
                vec![
                    col("id", true, None),
                    col("user_id", false, Some(("users", "id"))),
                ],
            ),
            table("audit", vec![col("id", true, None)]),
        ];
        let first = layout(&a, false);
        a.reverse();
        let second = layout(&a, false);

        let key = |d: &SchemaDiagramResp| -> Vec<(String, u64, u64)> {
            let mut v: Vec<_> = d
                .nodes
                .iter()
                .map(|n| (n.table.clone(), n.x as u64, n.y as u64))
                .collect();
            v.sort();
            v
        };
        assert_eq!(key(&first), key(&second));
        assert_eq!(first.crossings, second.crossings);
    }

    #[test]
    fn a_wide_table_is_summarised_rather_than_drawn_in_full() {
        let cols: Vec<ColumnDesc> = (0..30)
            .map(|i| col(&format!("c{i}"), i == 0, None))
            .collect();
        let d = layout(&[table("wide", cols)], false);
        let n = &d.nodes[0];
        assert_eq!(n.columns.len(), MAX_ROWS);
        assert_eq!(n.hidden_columns, 30 - MAX_ROWS);
        // Height reflects what is drawn, not the full column count.
        assert_eq!(n.h, node_height(MAX_ROWS));
    }

    #[test]
    fn a_reference_to_a_table_outside_the_set_is_dropped() {
        // The parent was filtered out or truncated away; an edge to nowhere
        // would render as a line into empty space.
        let d = layout(
            &[table(
                "orders",
                vec![col("user_id", false, Some(("users", "id")))],
            )],
            true,
        );
        assert!(d.edges.is_empty());
        assert_eq!(d.isolated, vec!["orders"]);
        assert!(d.truncated);
    }
}
