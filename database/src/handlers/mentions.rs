//! Chat mentions of connections: `@database(id="<name>")`.
//!
//! `database::mention::search` feeds the composer's `@database:` menu and
//! `database::mention::get` turns one connection name into the pill, the
//! preview card and the line an agent reads — which `db` to pass to
//! `database::query`, and where to list its tables. The get function
//! carries the provider descriptor in its metadata (see `mention-contract`);
//! agents reach the connection through `database::listTables` (the
//! descriptor's `details`), never through the mention functions.
//!
//! Only configuration metadata is read (name, driver, host, database,
//! folder, status): no connection is opened and nothing sensitive is shown.

use mention_contract::{
    color, tone, MentionField, MentionGetRequest, MentionItem, MentionOpen, MentionProvider,
    MentionSearchRequest, MentionSearchResponse, MentionView,
};
use serde_json::json;

use super::AppState;
use crate::config::{DatabaseConfig, DriverKind};

pub const SEARCH_FN: &str = "database::mention::search";
pub const GET_FN: &str = "database::mention::get";
pub const MENTION_NAME: &str = "database";
pub const ICON: &str = "database";

pub fn provider() -> MentionProvider {
    MentionProvider::new(MENTION_NAME, "Databases", SEARCH_FN)
        .description("A database connection, by name")
        .icon(ICON)
        .color(color::TEAL)
        .details("database::listTables", "db")
}

/// Search function metadata: plumbing, hidden from agents and traces.
pub fn search_metadata() -> serde_json::Value {
    json!({ "internal": true, "trace_hidden": true })
}

/// `host:port/db`, or the file path for sqlite.
fn location(db: &DatabaseConfig) -> String {
    match db.driver {
        DriverKind::Sqlite => db
            .database
            .clone()
            .or_else(|| {
                db.url
                    .as_deref()
                    .and_then(|u| u.strip_prefix("sqlite:"))
                    .map(str::to_string)
            })
            .unwrap_or_default(),
        _ => {
            let (host, port) = db.server_endpoint().unwrap_or_default();
            match db.database_name() {
                Some(name) => format!("{host}:{port}/{name}"),
                None => format!("{host}:{port}"),
            }
        }
    }
}

/// The nearest palette color to a `#RRGGBB` display color, by hue.
fn palette_color(hex: Option<&str>) -> Option<String> {
    let hex = hex?.strip_prefix('#')?;
    let channel = |i: usize| {
        u8::from_str_radix(hex.get(i..i + 2)?, 16)
            .ok()
            .map(f32::from)
    };
    let (r, g, b) = (
        channel(0)? / 255.0,
        channel(2)? / 255.0,
        channel(4)? / 255.0,
    );
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max - min < 0.12 {
        return Some(color::NEUTRAL.into());
    }
    let d = max - min;
    let hue = if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    Some(
        match hue {
            h if !(15.0..330.0).contains(&h) => color::ROSE,
            h if h < 70.0 => color::AMBER,
            h if h < 160.0 => color::GREEN,
            h if h < 200.0 => color::TEAL,
            h if h < 255.0 => color::BLUE,
            _ => color::PURPLE,
        }
        .into(),
    )
}

fn item(name: &str, db: &DatabaseConfig) -> MentionItem {
    MentionItem {
        id: name.to_string(),
        label: name.to_string(),
        // Chat surfaces show a mention as `@<hint>` (kanban's is the ticket
        // key): the connection's name, not its driver.
        hint: Some(name.to_string()),
        description: Some(description(db)),
        icon: Some(ICON.into()),
        color: palette_color(db.display.as_ref().and_then(|d| d.color.as_deref())),
    }
}

/// `postgres · host:port/db` (`sqlite · path`).
fn description(db: &DatabaseConfig) -> String {
    format!("{} · {}", db.driver, location(db))
}

pub async fn search(state: &AppState, req: MentionSearchRequest) -> MentionSearchResponse {
    let query = req.trimmed_query().to_lowercase();
    let cfg = state.config.read().await;
    let mut hits: Vec<(u8, String, MentionItem)> = cfg
        .databases
        .iter()
        .filter_map(|(name, db)| {
            let lname = name.to_lowercase();
            let rank = if query.is_empty() || lname.starts_with(&query) {
                0
            } else if lname.contains(&query) {
                1
            } else if format!("{} {}", db.driver, location(db))
                .to_lowercase()
                .contains(&query)
            {
                2
            } else {
                return None;
            };
            Some((rank, lname, item(name, db)))
        })
        .collect();
    hits.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    MentionSearchResponse {
        items: hits
            .into_iter()
            .take(req.effective_limit())
            .map(|(_, _, item)| item)
            .collect(),
    }
}

pub async fn get(state: &AppState, req: MentionGetRequest) -> Option<MentionView> {
    let cfg = state.config.read().await;
    let name = req.id.trim();
    let db = cfg.databases.get(name)?;
    let error = state.runtime.error(name);
    let mut fields = vec![
        MentionField::new("Driver", db.driver.to_string()),
        MentionField::new(
            if db.driver == DriverKind::Sqlite {
                "File"
            } else {
                "Server"
            },
            location(db),
        ),
    ];
    if let Some(ssh) = &db.ssh {
        fields.push(MentionField::new(
            "SSH",
            format!("{}@{}:{}", ssh.user, ssh.host, ssh.port),
        ));
    }
    if let Some(folder) = &db.folder {
        fields.push(MentionField::new("Queries folder", folder.clone()));
    }
    fields.push(match &error {
        Some(e) => MentionField::new("Status", e.message.clone()).tone(tone::DANGER),
        None => MentionField::new("Status", "available").tone(tone::SUCCESS),
    });
    // "Save a query" means a .sql file in the connection's folder (what the
    // apps list), never the console's snippet store.
    let folder_hint = match db.folder.as_deref() {
        Some(f) => format!(
            " Queries for this connection are .sql files in {f}; to save a query, write a \
             .sql file there with the file tools. database::saveQuery only stores a snippet \
             in the web console's list, not a file."
        ),
        None => " This connection has no queries folder, so there is nowhere to save a .sql \
                 file for it; database::saveQuery only stores a snippet in the web console's \
                 list, not a file."
            .to_string(),
    };
    let status = match &error {
        Some(e) => format!(" It is currently unavailable: {}.", e.message),
        None => String::new(),
    };
    let summary = format!(
        "Database connection \"{name}\" ({driver}, {location}). Pass db: \"{name}\" to the \
         database functions: database::query {{db: \"{name}\", sql}} for reads, \
         database::execute for writes, database::listTables {{db: \"{name}\"}} and \
         database::describeTable to explore the schema.{folder_hint}{status}",
        driver = db.driver,
        location = location(db),
    );
    Some(MentionView {
        id: name.to_string(),
        label: name.to_string(),
        hint: Some(name.to_string()),
        description: Some(description(db)),
        icon: Some(ICON.into()),
        color: palette_color(db.display.as_ref().and_then(|d| d.color.as_deref())),
        fields,
        open: Some(MentionOpen::Page {
            page: "database".into(),
            context: Some(json!({ "db": name })),
        }),
        summary: Some(summary),
        data: Some(json!({
            "name": name,
            "driver": db.driver,
            "host": db.server_endpoint().map(|(h, _)| h),
            "port": db.server_endpoint().map(|(_, p)| p),
            "database": db.database_name(),
            "folder": db.folder,
            "display": db.display,
            "error": error.map(|e| e.message),
        })),
        updated_at: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn state() -> AppState {
        let cfg = crate::config::WorkerConfig::from_yaml(
            "databases:\n  shop_pg:\n    driver: postgres\n    host: db.example\n    user: app\n    password: hunter2\n    database: shop\n    folder: /srv/q\n    display: { color: \"#336791\" }\n  analytics:\n    url: \"sqlite:/data/a.db\"\n  shop_mysql:\n    driver: mysql\n    host: my.example\n    user: app\n",
        )
        .unwrap();
        AppState {
            pools: Default::default(),
            config: Arc::new(RwLock::new(cfg)),
            handles: Arc::new(crate::handle::HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_helpers::observability::Logger::new(),
            row_changes: None,
            runtime: Default::default(),
        }
    }

    #[test]
    fn the_provider_descriptor_validates() {
        provider().validate().unwrap();
        let meta = provider().metadata();
        assert_eq!(meta["internal"], true);
        assert_eq!(
            meta["mention"]["details"]["function_id"],
            "database::listTables"
        );
        assert_eq!(meta["mention"]["details"]["id_field"], "db");
    }

    #[tokio::test]
    async fn search_ranks_prefix_matches_first_and_filters() {
        let st = state();
        let all = search(&st, MentionSearchRequest::default()).await;
        assert_eq!(all.items.len(), 3);
        let shop = search(
            &st,
            MentionSearchRequest {
                query: "shop".into(),
                ..Default::default()
            },
        )
        .await;
        let ids: Vec<&str> = shop.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["shop_mysql", "shop_pg"]);
        let by_host = search(
            &st,
            MentionSearchRequest {
                query: "my.example".into(),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(by_host.items[0].id, "shop_mysql");
        assert_eq!(
            shop.items[1].hint.as_deref(),
            Some("shop_pg"),
            "the pill reads @shop_pg"
        );
        assert_eq!(
            shop.items[1].description.as_deref(),
            Some("postgres · db.example:5432/shop")
        );
        assert_eq!(shop.items[1].color.as_deref(), Some("blue"));
    }

    #[tokio::test]
    async fn get_tells_an_agent_which_db_to_pass_and_never_shows_secrets() {
        let st = state();
        let view = get(
            &st,
            MentionGetRequest {
                id: "shop_pg".into(),
            },
        )
        .await
        .unwrap();
        let summary = view.summary.clone().unwrap();
        assert!(summary.contains("db: \"shop_pg\""), "{summary}");
        assert!(summary.contains("database::listTables"), "{summary}");
        assert!(
            summary.contains(
                "Queries for this connection are .sql files in /srv/q; to save a query, \
                 write a .sql file there with the file tools."
            ),
            "{summary}"
        );
        assert!(
            summary.contains("database::saveQuery only stores a snippet"),
            "{summary}"
        );
        // Without a folder it says so.
        let other = get(
            &st,
            MentionGetRequest {
                id: "shop_mysql".into(),
            },
        )
        .await
        .unwrap()
        .summary
        .unwrap();
        assert!(other.contains("has no queries folder"), "{other}");
        assert!(
            other.contains("database::saveQuery only stores a snippet"),
            "{other}"
        );
        assert!(!serde_json::to_string(&view).unwrap().contains("hunter2"));
        assert!(get(&st, MentionGetRequest { id: "nope".into() })
            .await
            .is_none());
    }

    #[test]
    fn display_colors_map_to_the_palette() {
        assert_eq!(palette_color(Some("#22C55E")).as_deref(), Some("green"));
        assert_eq!(palette_color(Some("#F29111")).as_deref(), Some("amber"));
        assert_eq!(palette_color(Some("#EF4444")).as_deref(), Some("rose"));
        assert_eq!(palette_color(Some("#808080")).as_deref(), Some("neutral"));
        assert_eq!(palette_color(Some("nope")), None);
    }
}
