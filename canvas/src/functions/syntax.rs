//! `canvas::syntax` — the mermaid syntax reference, on the bus.
//!
//! An agent writing mermaid should not guess dialect details from training
//! data; this returns the families the renderer actually supports, each with
//! a working example, and — narrowed to one family — a hand-written primer
//! dense enough to write a correct diagram from. The eight families agents
//! reach for most carry full primers; the rest get their one-line summary,
//! a valid starting skeleton, and nothing padded.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::functions::family;
use crate::store::Store;

pub const ID: &str = "canvas::syntax";
pub const DESC: &str = "Return the mermaid syntax reference: every supported diagram family with \
                        a short summary and a working example, or — narrowed to one family — a \
                        compact syntax primer. Call this before writing mermaid source.";

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Request {
    /// Only return this diagram family (`flowchart`, `sequenceDiagram`, …;
    /// aliases like `graph`, `sequence` or `state` are accepted). Omit for
    /// the one-line overview of every family.
    #[serde(default)]
    pub family: Option<String>,
}

/// One diagram family's reference entry.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FamilySyntax {
    /// Canonical family name (`flowchart`, `sequenceDiagram`, …) — the same
    /// string `canvas::validate` reports and `canvas::create` stores.
    pub family: String,

    /// One-line description of what this family is for.
    pub summary: String,

    /// A minimal, valid mermaid example of this family.
    pub example: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Response {
    /// The reference entries — every family for the overview, exactly one
    /// when the request named a family.
    pub families: Vec<FamilySyntax>,

    /// The reference as readable text: one line per family for the overview,
    /// or the named family's syntax primer with its example.
    pub syntax: String,
}

pub async fn handle(_store: &Store, req: Request, _cfg: &WorkerConfig) -> Result<Response, String> {
    match req
        .family
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
    {
        None => Ok(overview()),
        Some(input) => {
            let canonical = family::normalize(input).ok_or_else(|| {
                format!(
                    "unknown diagram family '{input}' — supported: {}",
                    family::FAMILIES.join(", ")
                )
            })?;
            Ok(one_family(canonical))
        }
    }
}

fn entry(family: &str) -> FamilySyntax {
    FamilySyntax {
        family: family.to_string(),
        summary: summary(family).to_string(),
        example: example(family).to_string(),
    }
}

fn overview() -> Response {
    let mut syntax = String::from(
        "Mermaid families this canvas renders (call canvas::syntax {\"family\": …} for a primer):\n",
    );
    for fam in family::FAMILIES {
        syntax.push_str(&format!("  {fam} — {}\n", summary(fam)));
    }
    Response {
        families: family::FAMILIES.iter().map(|f| entry(f)).collect(),
        syntax,
    }
}

fn one_family(canonical: &str) -> Response {
    let syntax = match primer(canonical) {
        Some(primer) => primer.to_string(),
        None => format!(
            "{canonical} — {}\n\nStart the source with the `{}` header line. Minimal example:\n\n{}\n\
             \nNo detailed primer is bundled for this family; the example above is a valid \
             skeleton to grow. Full parsing happens at render time in the console.",
            summary(canonical),
            header(canonical),
            example(canonical)
        ),
    };
    Response {
        families: vec![entry(canonical)],
        syntax,
    }
}

/// The header keyword that actually goes on line one — differs from the
/// canonical family name where mermaid still requires a suffixed keyword.
fn header(family: &str) -> &'static str {
    match family {
        "stateDiagram" => "stateDiagram-v2",
        "packet" => "packet-beta",
        "flowchart" => "flowchart TD",
        "sequenceDiagram" => "sequenceDiagram",
        "classDiagram" => "classDiagram",
        "erDiagram" => "erDiagram",
        "journey" => "journey",
        "gantt" => "gantt",
        "pie" => "pie",
        "quadrantChart" => "quadrantChart",
        "requirementDiagram" => "requirementDiagram",
        "gitGraph" => "gitGraph",
        "C4Context" => "C4Context",
        "mindmap" => "mindmap",
        "timeline" => "timeline",
        "kanban" => "kanban",
        "architecture-beta" => "architecture-beta",
        "block-beta" => "block-beta",
        "sankey-beta" => "sankey-beta",
        "xychart-beta" => "xychart-beta",
        "radar-beta" => "radar-beta",
        "treemap-beta" => "treemap-beta",
        _ => "%% unknown family",
    }
}

fn summary(family: &str) -> &'static str {
    match family {
        "flowchart" => "nodes and edges with automatic layout — processes, dependencies, decisions",
        "sequenceDiagram" => "actors exchanging messages over time — protocols, API call flows",
        "classDiagram" => "classes with members and typed relations — data models, OO design",
        "stateDiagram" => "states and transitions, with nesting and concurrency — lifecycles",
        "erDiagram" => "entities, attributes and crow's-foot relationships — database schemas",
        "journey" => "user journey steps scored per actor — experience mapping",
        "gantt" => "tasks on a calendar with dependencies and milestones — project plans",
        "pie" => "a labelled pie chart from value pairs",
        "quadrantChart" => "points plotted on a labelled 2x2 quadrant — prioritization",
        "requirementDiagram" => "requirements, elements and satisfies/verifies links — SysML-style",
        "gitGraph" => "commits, branches and merges drawn as a git history",
        "C4Context" => "C4 system-context diagram — people, systems and their relationships",
        "mindmap" => "a radial tree built from indentation — brainstorms, topic breakdowns",
        "timeline" => "events grouped along a time axis — history, roadmaps",
        "packet" => "byte/bit field layout of a network packet",
        "kanban" => "cards in columns — boards, work in progress",
        "architecture-beta" => "services, groups and connections with icons — cloud architecture",
        "block-beta" => "blocks on a column grid with manual placement — layered layouts",
        "sankey-beta" => "flow quantities between nodes as ribbons — energy, traffic, funnels",
        "xychart-beta" => "bar and/or line series on labelled x/y axes",
        "radar-beta" => "one or more value curves over shared axes — skill/feature comparison",
        "treemap-beta" => "nested proportional rectangles from a value tree — storage, budgets",
        _ => "not a mermaid family this worker knows",
    }
}

fn example(family: &str) -> &'static str {
    match family {
        "flowchart" => {
            "flowchart LR\n  U[User] -->|request| S(Server)\n  S --> D{cached?}\n  D -->|yes| C[(Cache)]\n  D -->|no| B[Build page]"
        }
        "sequenceDiagram" => {
            "sequenceDiagram\n  participant U as User\n  participant S as Server\n  U->>S: GET /home\n  S-->>U: 200 OK"
        }
        "classDiagram" => {
            "classDiagram\n  class Animal {\n    +name: string\n    +speak() void\n  }\n  Animal <|-- Dog\n  Animal <|-- Cat"
        }
        "stateDiagram" => {
            "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Running : start\n  Running --> Idle : stop\n  Running --> [*] : crash"
        }
        "erDiagram" => {
            "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  ORDER ||--|{ LINE-ITEM : contains\n  CUSTOMER {\n    string id PK\n    string email\n  }"
        }
        "journey" => {
            "journey\n  title Morning routine\n  section Getting up\n    Wake up: 3: Me\n    Coffee: 5: Me\n  section Commute\n    Train: 2: Me"
        }
        "gantt" => {
            "gantt\n  title Release plan\n  dateFormat YYYY-MM-DD\n  section Build\n    Design :d1, 2026-01-01, 5d\n    Implement :after d1, 10d\n  section Ship\n    Release :milestone, after d1, 0d"
        }
        "pie" => "pie title Languages\n  \"Rust\" : 45\n  \"TypeScript\" : 35\n  \"Other\" : 20",
        "quadrantChart" => {
            "quadrantChart\n  title Reach vs Effort\n  x-axis Low Reach --> High Reach\n  y-axis Low Effort --> High Effort\n  quadrant-1 Plan\n  quadrant-2 Do\n  quadrant-3 Skip\n  quadrant-4 Delegate\n  Feature A: [0.8, 0.3]"
        }
        "requirementDiagram" => {
            "requirementDiagram\n  requirement fast_response {\n    id: 1\n    text: respond under 100ms\n    risk: medium\n    verifymethod: test\n  }\n  element api {\n    type: service\n  }\n  api - satisfies -> fast_response"
        }
        "gitGraph" => {
            "gitGraph\n  commit\n  branch feature\n  checkout feature\n  commit\n  checkout main\n  merge feature"
        }
        "C4Context" => {
            "C4Context\n  title System context\n  Person(user, \"User\")\n  System(app, \"App\", \"Serves requests\")\n  System_Ext(mail, \"Mail service\")\n  Rel(user, app, \"Uses\")\n  Rel(app, mail, \"Sends mail via\")"
        }
        "mindmap" => "mindmap\n  root((Product))\n    Research\n      Interviews\n    Build\n      MVP",
        "timeline" => {
            "timeline\n  title Project history\n  2024 : Prototype\n  2025 : Launch : First customers\n  2026 : Scale"
        }
        "packet" => {
            "packet-beta\n  0-15: \"Source Port\"\n  16-31: \"Destination Port\"\n  32-63: \"Sequence Number\""
        }
        "kanban" => {
            "kanban\n  todo[To do]\n    t1[Write docs]\n  doing[In progress]\n    t2[Build worker]\n  done[Done]\n    t3[Design schema]"
        }
        "architecture-beta" => {
            "architecture-beta\n  group api(cloud)[API]\n  service web(internet)[Web]\n  service app(server)[App] in api\n  service db(database)[DB] in api\n  web:R --> L:app\n  app:B -- T:db"
        }
        "block-beta" => "block-beta\n  columns 3\n  a b c\n  d[\"wide block\"]:3",
        "sankey-beta" => "sankey-beta\n  Solar,Grid,30\n  Wind,Grid,50\n  Grid,Homes,60\n  Grid,Industry,20",
        "xychart-beta" => {
            "xychart-beta\n  title \"Revenue\"\n  x-axis [Q1, Q2, Q3, Q4]\n  y-axis \"USD (k)\" 0 --> 100\n  bar [20, 45, 60, 80]\n  line [20, 45, 60, 80]"
        }
        "radar-beta" => {
            "radar-beta\n  title Skills\n  axis r[\"Rust\"], t[\"TypeScript\"], s[\"SQL\"]\n  curve me[\"Me\"]{80, 70, 60}"
        }
        "treemap-beta" => {
            "treemap-beta\n\"Storage\"\n    \"Images\": 60\n    \"Video\": 30\n    \"Docs\": 10"
        }
        _ => "%% unknown family — canvas::syntax with no family lists the catalog",
    }
}

/// Hand-written ~30-line primers for the eight families agents write most.
/// Each ends with a small worked example so the syntax lands with a shape.
fn primer(family: &str) -> Option<&'static str> {
    let text = match family {
        "flowchart" => {
            "flowchart — nodes and edges, laid out automatically.\n\
             \n\
             Header: `flowchart TD` (top-down) | TB | BT | LR | RL. `graph` is an alias.\n\
             \n\
             Nodes — an id plus an optional shaped label:\n\
             \x20 A            plain node, label \"A\"\n\
             \x20 B[text]      rectangle\n\
             \x20 C(text)      rounded\n\
             \x20 D([text])    stadium\n\
             \x20 E{text}      diamond (decision)\n\
             \x20 F((text))    circle\n\
             \x20 G[(text)]    cylinder (database)\n\
             Quote labels holding special characters: H[\"a (b)\"]\n\
             \n\
             Edges:\n\
             \x20 A --> B          arrow\n\
             \x20 A --- B          plain line\n\
             \x20 A -.-> B         dotted arrow\n\
             \x20 A ==> B          thick arrow\n\
             \x20 A -->|label| B   labelled edge (or A -- label --> B)\n\
             Chains: A --> B --> C   fan-out: A --> B & C\n\
             \n\
             Subgraphs:\n\
             \x20 subgraph Backend\n\
             \x20   S --> D\n\
             \x20 end\n\
             \n\
             Styling: classDef hot fill:#f96;  class A hot;  comments start with %%.\n\
             \n\
             Example:\n\
             flowchart LR\n\
             \x20 U[User] -->|request| S(Server)\n\
             \x20 S --> D{cached?}\n\
             \x20 D -->|yes| C[(Cache)]\n\
             \x20 D -->|no| B[Build page]"
        }
        "sequenceDiagram" => {
            "sequenceDiagram — actors exchanging messages over time.\n\
             \n\
             Header: `sequenceDiagram`, then optional declarations — order fixes the\n\
             columns: `participant A as Alice` (box) or `actor B as Bob` (person).\n\
             \n\
             Messages (solid = request, dashed = response):\n\
             \x20 A->>B: text      solid arrowhead\n\
             \x20 B-->>A: text     dashed arrowhead\n\
             \x20 A-)B: text       async (open arrow)\n\
             \x20 A-xB: text       lost / failed (cross)\n\
             \n\
             Activations — bars on a lifeline:\n\
             \x20 activate B / deactivate B, or shorthand A->>+B: … then B-->>-A: …\n\
             \n\
             Grouping blocks (all closed with `end`):\n\
             \x20 loop every 30s … end\n\
             \x20 alt ok … else error … end\n\
             \x20 opt only sometimes … end\n\
             \x20 par branch A … and branch B … end\n\
             \n\
             Notes: `Note right of A: text` | `Note over A,B: text`\n\
             `autonumber` numbers every message. Comments start with %%.\n\
             \n\
             Example:\n\
             sequenceDiagram\n\
             \x20 autonumber\n\
             \x20 participant U as User\n\
             \x20 participant S as Server\n\
             \x20 U->>+S: GET /home\n\
             \x20 S-->>-U: 200 OK\n\
             \x20 Note over U,S: cached after first hit"
        }
        "classDiagram" => {
            "classDiagram — classes, members and typed relations.\n\
             \n\
             Class block — visibility prefixes: + public, - private, # protected:\n\
             \x20 class Order {\n\
             \x20   +id: string\n\
             \x20   -items: Item[]\n\
             \x20   +total() float\n\
             \x20 }\n\
             One-liners work too: `Order : +ship()`\n\
             \n\
             Relations (read left to right):\n\
             \x20 A <|-- B    inheritance (B extends A)\n\
             \x20 A *-- B     composition\n\
             \x20 A o-- B     aggregation\n\
             \x20 A --> B     association\n\
             \x20 A ..> B     dependency\n\
             \x20 A ..|> B    interface realization\n\
             Cardinality and label: A \"1\" --> \"many\" B : contains\n\
             \n\
             Annotations inside or above a class:\n\
             \x20 <<interface>> Shape,  <<enumeration>> Color,  <<abstract>> Base\n\
             Generics use tildes: class Box~T~. Comments start with %%.\n\
             \n\
             Example:\n\
             classDiagram\n\
             \x20 class Animal {\n\
             \x20   +name: string\n\
             \x20   +speak() void\n\
             \x20 }\n\
             \x20 Animal <|-- Dog\n\
             \x20 Animal \"1\" --> \"*\" Meal : eats"
        }
        "stateDiagram" => {
            "stateDiagram — states and transitions. Use the `stateDiagram-v2` header.\n\
             \n\
             `[*]` is both the start and end marker:\n\
             \x20 [*] --> Idle\n\
             \x20 Idle --> Running : start\n\
             \x20 Running --> [*]\n\
             The label after the colon is the event; add a guard in brackets:\n\
             \x20 Running --> Halted : error [fatal]\n\
             \n\
             Long names: `state \"Waiting for input\" as Waiting`\n\
             \n\
             Composite (nested) states:\n\
             \x20 state Running {\n\
             \x20   [*] --> Warming\n\
             \x20   Warming --> Steady\n\
             \x20 }\n\
             \n\
             Choice, fork and join pseudo-states:\n\
             \x20 state check <<choice>>\n\
             \x20 state split <<fork>>\n\
             \x20 state merge <<join>>\n\
             Concurrency: a `--` line inside a composite state splits parallel regions.\n\
             \n\
             Notes: `note right of Idle: waiting for work`. Comments start with %%.\n\
             \n\
             Example:\n\
             stateDiagram-v2\n\
             \x20 [*] --> Idle\n\
             \x20 Idle --> Running : start\n\
             \x20 Running --> Idle : stop\n\
             \x20 Running --> [*] : crash"
        }
        "erDiagram" => {
            "erDiagram — entities, attributes and crow's-foot relationships.\n\
             \n\
             Relationship line: `LEFT <card><line><card> RIGHT : label`\n\
             Cardinality symbols (outer side faces the entity):\n\
             \x20 ||   exactly one\n\
             \x20 o|   zero or one\n\
             \x20 }|   one or more\n\
             \x20 }o   zero or more\n\
             Line style: `--` identifying, `..` non-identifying.\n\
             \n\
             \x20 CUSTOMER ||--o{ ORDER : places\n\
             \x20 ORDER ||--|{ LINE-ITEM : contains\n\
             \n\
             Attributes in a block — first word is the type; PK / FK / UK mark keys,\n\
             a trailing quoted string is a comment:\n\
             \x20 CUSTOMER {\n\
             \x20   string id PK\n\
             \x20   string email UK\n\
             \x20   int loyalty_points \"nullable\"\n\
             \x20 }\n\
             \n\
             Entity names are conventionally UPPERCASE. Comments start with %%.\n\
             \n\
             Example:\n\
             erDiagram\n\
             \x20 CUSTOMER ||--o{ ORDER : places\n\
             \x20 ORDER ||--|{ LINE-ITEM : contains\n\
             \x20 CUSTOMER {\n\
             \x20   string id PK\n\
             \x20   string email\n\
             \x20 }"
        }
        "gantt" => {
            "gantt — tasks on a calendar.\n\
             \n\
             Header block:\n\
             \x20 gantt\n\
             \x20   title Release plan\n\
             \x20   dateFormat YYYY-MM-DD\n\
             \x20   excludes weekends\n\
             \n\
             Tasks live under sections. Task line: `name : [tags,] [id,] start, duration`\n\
             \x20 section Build\n\
             \x20   Design    :done,    des1, 2026-01-01, 5d\n\
             \x20   Implement :active,  imp1, after des1, 10d\n\
             \x20   Review    :         rev1, after imp1, 3d\n\
             \x20 section Ship\n\
             \x20   Release   :milestone, rel1, after rev1, 0d\n\
             \n\
             Pieces:\n\
             \x20 tags      done | active | crit | milestone (combinable: crit, done)\n\
             \x20 start     a date, `after <id>`, or `until <id>`\n\
             \x20 duration  5d, 2w, 12h — or an explicit end date\n\
             The id is what `after` references; omit it for tasks nothing depends on.\n\
             Comments start with %%.\n\
             \n\
             Example:\n\
             gantt\n\
             \x20 title Plan\n\
             \x20 dateFormat YYYY-MM-DD\n\
             \x20 section Build\n\
             \x20   Design :d1, 2026-01-01, 5d\n\
             \x20   Implement :after d1, 10d"
        }
        "architecture-beta" => {
            "architecture-beta — services, groups and connections (cloud diagrams).\n\
             \n\
             Groups — `group <id>(icon)[Title]`, nested with `in`:\n\
             \x20 group api(cloud)[API]\n\
             \x20 group data(database)[Data] in api\n\
             \n\
             Services — `service <id>(icon)[Title] [in <group>]`:\n\
             \x20 service web(internet)[Web]\n\
             \x20 service app(server)[App] in api\n\
             \x20 service pg(database)[Postgres] in data\n\
             \n\
             Edges connect SIDES of two services — T, B, L or R:\n\
             \x20 web:R -- L:app       plain connection\n\
             \x20 web:R --> L:app      with an arrowhead\n\
             \x20 app:B -- T:pg\n\
             A `{group}` suffix lifts the edge to the service's group border:\n\
             \x20 app{group}:B -- T:ext\n\
             \n\
             Junctions let edges meet at a crossroads: `junction j1` then edge to j1.\n\
             Built-in icons: cloud, database, disk, internet, server.\n\
             Comments start with %%.\n\
             \n\
             Example:\n\
             architecture-beta\n\
             \x20 group api(cloud)[API]\n\
             \x20 service web(internet)[Web]\n\
             \x20 service app(server)[App] in api\n\
             \x20 service db(database)[DB] in api\n\
             \x20 web:R --> L:app\n\
             \x20 app:B -- T:db"
        }
        "mindmap" => {
            "mindmap — a radial tree built purely from indentation.\n\
             \n\
             The single root goes first; every deeper indent nests one level:\n\
             \x20 mindmap\n\
             \x20   root((Product))\n\
             \x20     Research\n\
             \x20       Interviews\n\
             \x20       Surveys\n\
             \x20     Build\n\
             \x20       MVP\n\
             \n\
             Node shapes wrap the text:\n\
             \x20 id[square]   id(rounded)   id((circle))\n\
             \x20 id))bang((   id)cloud(     id{{hexagon}}\n\
             Plain text gets the default shape.\n\
             \n\
             Decorations on the line below a node:\n\
             \x20 ::icon(fa fa-book)          an icon class\n\
             Markdown-ish `**bold**` and `*italic*` work inside labels.\n\
             \n\
             Indentation is the ONLY structure: keep it consistent (two spaces per\n\
             level) and never skip a level, or the tree reparents silently.\n\
             Comments start with %%.\n\
             \n\
             Example:\n\
             mindmap\n\
             \x20 root((Launch))\n\
             \x20   Research\n\
             \x20     Interviews\n\
             \x20   Build\n\
             \x20     MVP"
        }
        _ => return None,
    };
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::functions::family;

    async fn run(req: Request) -> Result<Response, String> {
        let store = Store::in_memory();
        handle(&store, req, &WorkerConfig::default()).await
    }

    #[tokio::test]
    async fn the_overview_covers_every_family_once() {
        let out = run(Request::default()).await.expect("answers");
        assert_eq!(out.families.len(), family::FAMILIES.len());
        for fam in family::FAMILIES {
            assert!(
                out.families.iter().any(|e| e.family == *fam),
                "{fam} missing from the entries"
            );
            assert!(out.syntax.contains(fam), "{fam} missing from the text");
        }
    }

    /// Every shipped example must detect as the family it documents — the
    /// reference and the detector may never disagree.
    #[tokio::test]
    async fn every_example_detects_as_its_own_family() {
        let out = run(Request::default()).await.expect("answers");
        for entry in &out.families {
            assert_eq!(
                family::detect(&entry.example).as_deref(),
                Some(entry.family.as_str()),
                "example for {} does not detect as {}:\n{}",
                entry.family,
                entry.family,
                entry.example
            );
        }
    }

    #[tokio::test]
    async fn the_eight_primer_families_answer_with_a_dense_primer() {
        for fam in [
            "flowchart",
            "sequenceDiagram",
            "classDiagram",
            "stateDiagram",
            "erDiagram",
            "gantt",
            "architecture-beta",
            "mindmap",
        ] {
            let out = run(Request {
                family: Some(fam.into()),
            })
            .await
            .expect("answers");
            assert_eq!(out.families.len(), 1);
            assert_eq!(out.families[0].family, fam);
            let lines = out.syntax.lines().count();
            assert!(
                lines >= 25,
                "{fam} primer is only {lines} lines — it must earn its tokens"
            );
        }
    }

    #[tokio::test]
    async fn other_families_get_the_generic_fallback_not_padding() {
        let out = run(Request {
            family: Some("pie".into()),
        })
        .await
        .expect("answers");
        assert_eq!(out.families[0].family, "pie");
        assert!(out.syntax.contains("pie"));
        assert!(out.syntax.contains("render time"));
    }

    #[tokio::test]
    async fn aliases_resolve_and_unknown_families_error_with_the_list() {
        let aliased = run(Request {
            family: Some("graph".into()),
        })
        .await
        .expect("answers");
        assert_eq!(aliased.families[0].family, "flowchart");

        let err = run(Request {
            family: Some("uml".into()),
        })
        .await
        .expect_err("unknown family");
        assert!(err.contains("uml"), "{err}");
        assert!(err.contains("flowchart"), "{err}");
    }
}
