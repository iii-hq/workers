//! Projects and releases: the derived views, the mutations, shipping and the
//! label conversion. Pure functions over the [`Board`], like [`crate::board`];
//! the handlers in [`crate::functions`] run them under the store lock.
//!
//! A ticket has at most one project and one release, independent of each
//! other and of its parent. A project or release stores only its own fields;
//! counts and status are derived from the live tickets that reference it.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::board::{
    BoardContext, find_project, find_release, key_number, project_name, record, release_name,
    trimmed,
};
use crate::store::{Board, Change, Project, Release, Ticket, default_actor};

/// The glyph tones a project may carry.
pub const PROJECT_COLORS: [&str; 6] = ["blue", "purple", "teal", "green", "amber", "rose"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Active,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseStatus {
    Planned,
    Active,
    Released,
}

/// Live tickets per column id (every configured column, zero included).
pub type Counts = BTreeMap<String, u64>;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: Project,
    pub status: ProjectStatus,
    /// Live tickets in the project.
    pub total: u64,
    /// Of those, the ones in the done column.
    pub done: u64,
    pub counts: Counts,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ReleaseProjectCount {
    /// `null` for tickets in no project.
    pub project_id: Option<String>,
    pub total: u64,
    pub done: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ReleaseView {
    #[serde(flatten)]
    pub release: Release,
    /// released: `released_at` is set. active: some live ticket is outside the
    /// first column. planned: otherwise.
    pub status: ReleaseStatus,
    pub total: u64,
    pub done: u64,
    pub counts: Counts,
    /// Not-done live tickets of a released release (0 while it is not released).
    pub reopened: u64,
    /// Per-project breakdown, biggest first; `project_id: null` is "no project".
    pub projects: Vec<ReleaseProjectCount>,
}

/// A project or a release, as `kanban::label::convert` returns it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum EntityView {
    Project(ProjectView),
    Release(ReleaseView),
}

/* ── views ────────────────────────────────────────────────────────────── */

fn live<'a>(
    board: &'a Board,
    keep: impl Fn(&Ticket) -> bool + 'a,
) -> impl Iterator<Item = &'a Ticket> {
    board
        .tickets
        .values()
        .filter(move |ticket| ticket.deleted_at.is_none() && keep(ticket))
}

fn tally<'a>(
    tickets: impl Iterator<Item = &'a Ticket>,
    context: &BoardContext,
) -> (u64, u64, Counts) {
    let mut counts: Counts = context
        .columns
        .iter()
        .map(|column| (column.id.clone(), 0))
        .collect();
    let (mut total, mut done) = (0, 0);
    for ticket in tickets {
        total += 1;
        done += u64::from(ticket.status == context.done_status);
        *counts.entry(ticket.status.clone()).or_default() += 1;
    }
    (total, done, counts)
}

pub fn project_view(board: &Board, project: &Project, context: &BoardContext) -> ProjectView {
    // ponytail: one scan of the board per project; fine for tens of projects, group in one pass beyond that.
    let id = project.id.clone();
    let (total, done, counts) = tally(
        live(board, move |ticket| {
            ticket.project_id.as_deref() == Some(id.as_str())
        }),
        context,
    );
    ProjectView {
        status: if project.archived_at.is_some() {
            ProjectStatus::Archived
        } else {
            ProjectStatus::Active
        },
        project: project.clone(),
        total,
        done,
        counts,
    }
}

pub fn release_view(board: &Board, release: &Release, context: &BoardContext) -> ReleaseView {
    let id = release.id.clone();
    let members: Vec<&Ticket> = live(board, move |ticket| {
        ticket.release_id.as_deref() == Some(id.as_str())
    })
    .collect();
    let (total, done, counts) = tally(members.iter().copied(), context);
    let first = context.columns.first().map(|column| column.id.as_str());
    let status = if release.released_at.is_some() {
        ReleaseStatus::Released
    } else if members
        .iter()
        .any(|ticket| Some(ticket.status.as_str()) != first)
    {
        ReleaseStatus::Active
    } else {
        ReleaseStatus::Planned
    };
    let mut per_project: HashMap<Option<&str>, (u64, u64)> = HashMap::new();
    for ticket in &members {
        let entry = per_project.entry(ticket.project_id.as_deref()).or_default();
        entry.0 += 1;
        entry.1 += u64::from(ticket.status == context.done_status);
    }
    let name_of = |id: Option<&str>| {
        id.and_then(|id| board.projects.get(id))
            .map(|project| project.name.to_ascii_lowercase())
    };
    let mut projects: Vec<ReleaseProjectCount> = per_project
        .into_iter()
        .map(|(project_id, (total, done))| ReleaseProjectCount {
            project_id: project_id.map(String::from),
            total,
            done,
        })
        .collect();
    projects.sort_by(|a, b| {
        b.total.cmp(&a.total).then_with(|| {
            let (a_name, b_name) = (
                name_of(a.project_id.as_deref()),
                name_of(b.project_id.as_deref()),
            );
            // Named projects before "no project", then by name.
            a_name
                .is_none()
                .cmp(&b_name.is_none())
                .then_with(|| a_name.cmp(&b_name))
        })
    });
    ReleaseView {
        status,
        reopened: if release.released_at.is_some() {
            total - done
        } else {
            0
        },
        release: release.clone(),
        total,
        done,
        counts,
        projects,
    }
}

/// Case-insensitive comparison where a run of digits compares by value:
/// "Phase 2" sorts before "Phase 10". Ties fall back to the plain order so
/// the result is total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
        let mut run = String::new();
        while let Some(c) = chars.next_if(char::is_ascii_digit) {
            run.push(c);
        }
        run
    }
    let (mut left, mut right) = (a.chars().peekable(), b.chars().peekable());
    loop {
        let order = match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (x, y) = (digits(&mut left), digits(&mut right));
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            (Some(x), Some(y)) => {
                left.next();
                right.next();
                x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase())
            }
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

/// Projects by name; archived ones only when asked for.
pub fn project_views(
    board: &Board,
    context: &BoardContext,
    include_archived: bool,
) -> Vec<ProjectView> {
    let mut projects: Vec<&Project> = board
        .projects
        .values()
        .filter(|project| include_archived || project.archived_at.is_none())
        .collect();
    projects.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    projects
        .into_iter()
        .map(|project| project_view(board, project, context))
        .collect()
}

/// Releases by target date (none last), then by name, "Phase 2" before "Phase 10".
pub fn release_views(board: &Board, context: &BoardContext) -> Vec<ReleaseView> {
    let mut releases: Vec<&Release> = board.releases.values().collect();
    releases.sort_by(|a, b| {
        a.target_at
            .is_none()
            .cmp(&b.target_at.is_none())
            .then_with(|| a.target_at.cmp(&b.target_at))
            .then_with(|| natural_cmp(&a.name, &b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    releases
        .into_iter()
        .map(|release| release_view(board, release, context))
        .collect()
}

/* ── validation ───────────────────────────────────────────────────────── */

fn clean_name(raw: &str, kind: &str) -> Result<String, String> {
    trimmed(Some(raw)).ok_or_else(|| format!("{kind} name is required"))
}

/// `name` must differ from every other name of the kind, and from every other
/// id: a reference is an id or a name, so a name that spells another entity's
/// id would shadow it. (ASCII case ignored, like the lookup.)
fn ensure_free<T>(
    entities: &indexmap::IndexMap<String, T>,
    name: &str,
    except: Option<&str>,
    kind: &str,
    id_of: fn(&T) -> &str,
    name_of: fn(&T) -> &str,
) -> Result<(), String> {
    for entity in entities
        .values()
        .filter(|entity| Some(id_of(entity)) != except)
    {
        if name_of(entity).trim().eq_ignore_ascii_case(name) {
            return Err(format!("a {kind} named {name} already exists"));
        }
        if id_of(entity).eq_ignore_ascii_case(name) {
            return Err(format!(
                "{name} is the id of another {kind}; choose a different name"
            ));
        }
    }
    Ok(())
}

fn clean_color(raw: &str) -> Result<Option<String>, String> {
    let Some(color) = trimmed(Some(raw)) else {
        return Ok(None);
    };
    if PROJECT_COLORS.contains(&color.as_str()) {
        return Ok(Some(color));
    }
    Err(format!(
        "unknown color: {color} (use one of {})",
        PROJECT_COLORS.join(", ")
    ))
}

/// `YYYY-MM-DD`, a real calendar day, nothing else.
fn clean_target(raw: &str) -> Result<Option<String>, String> {
    let Some(raw) = trimmed(Some(raw)) else {
        return Ok(None);
    };
    match chrono::NaiveDate::parse_from_str(&raw, "%Y-%m-%d") {
        Ok(day) if day.to_string() == raw => Ok(Some(raw)),
        _ => Err(format!("target_at must be a date as YYYY-MM-DD, got {raw}")),
    }
}

/* ── projects ─────────────────────────────────────────────────────────── */

/// Fields of `kanban::project::create` / `update`. `None` = leave alone;
/// `Some(None)` = clear (`lead`, `color`).
#[derive(Debug, Clone, Default)]
pub struct ProjectFields {
    pub name: Option<String>,
    pub description: Option<String>,
    pub lead: Option<Option<String>>,
    pub color: Option<Option<String>>,
}

pub fn create_project(
    board: &mut Board,
    fields: &ProjectFields,
    context: &BoardContext,
    at: &str,
) -> Result<ProjectView, String> {
    let name = clean_name(fields.name.as_deref().unwrap_or_default(), "project")?;
    ensure_free(
        &board.projects,
        &name,
        None,
        "project",
        |p| &p.id,
        |p| &p.name,
    )?;
    let color = match &fields.color {
        Some(Some(color)) => clean_color(color)?,
        _ => None,
    };
    let project = Project {
        id: crate::board::new_id(),
        name,
        description: fields
            .description
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string(),
        lead: fields
            .lead
            .as_ref()
            .and_then(|lead| trimmed(lead.as_deref())),
        color,
        source_label: None,
        created_at: at.to_string(),
        updated_at: at.to_string(),
        archived_at: None,
    };
    let view = project_view(board, &project, context);
    board.projects.insert(project.id.clone(), project);
    Ok(view)
}

fn project_ref<'a>(board: &'a Board, reference: &str) -> Result<&'a Project, String> {
    find_project(board, reference).ok_or_else(|| format!("project not found: {reference}"))
}

pub fn update_project(
    board: &mut Board,
    reference: &str,
    fields: &ProjectFields,
    context: &BoardContext,
    at: &str,
) -> Result<ProjectView, String> {
    let id = project_ref(board, reference)?.id.clone();
    let name = match &fields.name {
        Some(name) => {
            let name = clean_name(name, "project")?;
            ensure_free(
                &board.projects,
                &name,
                Some(&id),
                "project",
                |p| &p.id,
                |p| &p.name,
            )?;
            Some(name)
        }
        None => None,
    };
    let color = match &fields.color {
        Some(Some(color)) => Some(clean_color(color)?),
        Some(None) => Some(None),
        None => None,
    };
    let project = board.projects.get_mut(&id).expect("resolved id exists");
    let mut changed = false;
    let mut set = |field: &mut String, value: Option<String>| {
        if let Some(value) = value.filter(|value| *value != *field) {
            *field = value;
            changed = true;
        }
    };
    set(&mut project.name, name);
    set(
        &mut project.description,
        fields.description.as_ref().map(|d| d.trim().to_string()),
    );
    let mut set_optional = |field: &mut Option<String>, value: Option<Option<String>>| {
        if let Some(value) = value.filter(|value| *value != *field) {
            *field = value;
            changed = true;
        }
    };
    set_optional(
        &mut project.lead,
        fields.lead.as_ref().map(|lead| trimmed(lead.as_deref())),
    );
    set_optional(&mut project.color, color);
    if changed {
        project.updated_at = at.to_string();
    }
    Ok(project_view(board, &board.projects[&id], context))
}

/// Archive (or restore) a project. Doing it twice changes nothing.
pub fn set_project_archived(
    board: &mut Board,
    reference: &str,
    archived: bool,
    context: &BoardContext,
    at: &str,
) -> Result<ProjectView, String> {
    let id = project_ref(board, reference)?.id.clone();
    let project = board.projects.get_mut(&id).expect("resolved id exists");
    if project.archived_at.is_some() != archived {
        project.archived_at = archived.then(|| at.to_string());
        project.updated_at = at.to_string();
    }
    Ok(project_view(board, &board.projects[&id], context))
}

/* ── releases ─────────────────────────────────────────────────────────── */

/// Fields of `kanban::release::create` / `update`. `None` = leave alone;
/// `Some(None)` = clear the target date.
#[derive(Debug, Clone, Default)]
pub struct ReleaseFields {
    pub name: Option<String>,
    pub description: Option<String>,
    pub target_at: Option<Option<String>>,
}

pub fn create_release(
    board: &mut Board,
    fields: &ReleaseFields,
    context: &BoardContext,
    at: &str,
) -> Result<ReleaseView, String> {
    let name = clean_name(fields.name.as_deref().unwrap_or_default(), "release")?;
    ensure_free(
        &board.releases,
        &name,
        None,
        "release",
        |r| &r.id,
        |r| &r.name,
    )?;
    let target_at = match &fields.target_at {
        Some(Some(day)) => clean_target(day)?,
        _ => None,
    };
    let release = Release {
        id: crate::board::new_id(),
        name,
        description: fields
            .description
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string(),
        target_at,
        released_at: None,
        source_label: None,
        created_at: at.to_string(),
        updated_at: at.to_string(),
    };
    let view = release_view(board, &release, context);
    board.releases.insert(release.id.clone(), release);
    Ok(view)
}

fn release_ref<'a>(board: &'a Board, reference: &str) -> Result<&'a Release, String> {
    find_release(board, reference).ok_or_else(|| format!("release not found: {reference}"))
}

pub fn update_release(
    board: &mut Board,
    reference: &str,
    fields: &ReleaseFields,
    context: &BoardContext,
    at: &str,
) -> Result<ReleaseView, String> {
    let id = release_ref(board, reference)?.id.clone();
    let name = match &fields.name {
        Some(name) => {
            let name = clean_name(name, "release")?;
            ensure_free(
                &board.releases,
                &name,
                Some(&id),
                "release",
                |r| &r.id,
                |r| &r.name,
            )?;
            Some(name)
        }
        None => None,
    };
    let target_at = match &fields.target_at {
        Some(Some(day)) => Some(clean_target(day)?),
        Some(None) => Some(None),
        None => None,
    };
    let release = board.releases.get_mut(&id).expect("resolved id exists");
    let mut changed = false;
    if let Some(name) = name.filter(|name| *name != release.name) {
        release.name = name;
        changed = true;
    }
    if let Some(description) = fields.description.as_ref().map(|d| d.trim().to_string())
        && description != release.description
    {
        release.description = description;
        changed = true;
    }
    if let Some(target_at) = target_at.filter(|target_at| *target_at != release.target_at) {
        release.target_at = target_at;
        changed = true;
    }
    if changed {
        release.updated_at = at.to_string();
    }
    Ok(release_view(board, &board.releases[&id], context))
}

/// Mark a release as released. Live tickets that are not done must go
/// somewhere: `move_open_to` is `Some(Some(release))` to move them,
/// `Some(None)` to take them out of any release, `None` when the caller did
/// not choose (an error if there are any). Returns the release and the ids of
/// the tickets moved.
pub fn ship_release(
    board: &mut Board,
    reference: &str,
    move_open_to: Option<Option<String>>,
    context: &BoardContext,
    at: &str,
) -> Result<(ReleaseView, Vec<String>), String> {
    let release = release_ref(board, reference)?;
    let (id, name) = (release.id.clone(), release.name.clone());
    if release.released_at.is_some() {
        return Err(format!("release {name} is already released"));
    }
    let mut open: Vec<String> = live(board, |ticket| {
        ticket.release_id.as_deref() == Some(id.as_str()) && ticket.status != context.done_status
    })
    .map(|ticket| ticket.id.clone())
    .collect();
    open.sort_by_key(|id| key_number(&board.tickets[id]));
    // Where the open tickets go matters only when there are some.
    let target = match &move_open_to {
        _ if open.is_empty() => None,
        None => {
            return Err(format!(
                "{} tickets are not done; choose where they go (move_open_to)",
                open.len()
            ));
        }
        Some(None) => None,
        Some(Some(wanted)) => {
            let target = trimmed(Some(wanted))
                .and_then(|wanted| find_release(board, &wanted))
                .ok_or_else(|| format!("release not found: {wanted}"))?;
            if target.id == id {
                return Err(format!("cannot move the open tickets into {name} itself"));
            }
            if target.released_at.is_some() {
                return Err(format!("release {} is already released", target.name));
            }
            Some((target.id.clone(), target.name.clone()))
        }
    };
    for ticket_id in &open {
        let ticket = board
            .tickets
            .get_mut(ticket_id)
            .expect("listed ticket exists");
        ticket.release_id = target.as_ref().map(|(id, _)| id.clone());
        ticket.updated_at = at.to_string();
        let change = Change {
            field: "release".into(),
            from: json!(name),
            to: json!(target.as_ref().map(|(_, name)| name)),
        };
        record(
            ticket,
            "ticket.updated",
            &default_actor(),
            at,
            vec![change],
            None,
        );
    }
    let release = board.releases.get_mut(&id).expect("resolved id exists");
    release.released_at = Some(at.to_string());
    release.updated_at = at.to_string();
    Ok((release_view(board, &board.releases[&id], context), open))
}

/// Clear `released_at`. Tickets moved out when it shipped stay where they went.
/// Reopening a release that is not released changes nothing.
pub fn reopen_release(
    board: &mut Board,
    reference: &str,
    context: &BoardContext,
    at: &str,
) -> Result<ReleaseView, String> {
    let id = release_ref(board, reference)?.id.clone();
    let release = board.releases.get_mut(&id).expect("resolved id exists");
    if release.released_at.take().is_some() {
        release.updated_at = at.to_string();
    }
    Ok(release_view(board, &board.releases[&id], context))
}

/// Delete a release nothing references. Soft-deleted tickets count: restoring
/// one must not leave it pointing at a release that is gone.
pub fn delete_release(board: &mut Board, reference: &str) -> Result<(), String> {
    let release = release_ref(board, reference)?;
    let used = board
        .tickets
        .values()
        .filter(|ticket| ticket.release_id.as_deref() == Some(release.id.as_str()))
        .count();
    if used > 0 {
        return Err(format!(
            "release {} still has {used} tickets (deleted ones included); move them to another release first",
            release.name
        ));
    }
    let id = release.id.clone();
    board.releases.shift_remove(&id);
    Ok(())
}

/* ── label conversion ─────────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConvertInto {
    Project,
    Release,
}

#[derive(Debug, Clone)]
pub struct ConvertInput {
    pub label: String,
    pub into: ConvertInto,
    /// An existing project or release (id or name).
    pub id: Option<String>,
    /// Without `id`: the project or release with this name, created when absent.
    pub name: Option<String>,
    pub description: Option<String>,
    pub target_at: Option<String>,
    /// Move tickets that sit in another project or release.
    pub replace: bool,
    /// Take the label off the tickets that end up in the project or release.
    pub remove_label: bool,
    /// Releases: mark it released when every live ticket in it is done.
    pub mark_released_if_done: bool,
}

/// Ticket keys per outcome. A bucket describes where a ticket was before the
/// conversion; with `replace`, `in_other` tickets are moved all the same.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Buckets {
    pub added: Vec<String>,
    pub already_here: Vec<String>,
    pub in_other: Vec<String>,
    pub deleted: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    Added,
    AlreadyHere,
    InOther,
    Deleted,
}

#[derive(Debug, Clone)]
enum Target {
    Existing(String),
    New {
        name: String,
        description: String,
        target_at: Option<String>,
    },
}

/// What a conversion would do, computed from the board without writing.
#[derive(Debug, Clone)]
pub struct ConvertPlan {
    target: Target,
    label: String,
    into: ConvertInto,
    rows: Vec<(String, Bucket)>,
}

impl ConvertPlan {
    /// The project or release the tickets go to, when it exists already.
    pub fn existing(&self) -> Option<&str> {
        match &self.target {
            Target::Existing(id) => Some(id),
            Target::New { .. } => None,
        }
    }

    pub fn buckets(&self, board: &Board) -> Buckets {
        let mut buckets = Buckets::default();
        for (id, bucket) in &self.rows {
            let key = board.tickets[id].key.clone();
            match bucket {
                Bucket::Added => buckets.added.push(key),
                Bucket::AlreadyHere => buckets.already_here.push(key),
                Bucket::InOther => buckets.in_other.push(key),
                Bucket::Deleted => buckets.deleted.push(key),
            }
        }
        buckets
    }
}

pub fn entity_view(
    board: &Board,
    into: ConvertInto,
    id: &str,
    context: &BoardContext,
) -> Option<EntityView> {
    match into {
        ConvertInto::Project => board
            .projects
            .get(id)
            .map(|project| EntityView::Project(project_view(board, project, context))),
        ConvertInto::Release => board
            .releases
            .get(id)
            .map(|release| EntityView::Release(release_view(board, release, context))),
    }
}

fn relation(ticket: &Ticket, into: ConvertInto) -> Option<&str> {
    match into {
        ConvertInto::Project => ticket.project_id.as_deref(),
        ConvertInto::Release => ticket.release_id.as_deref(),
    }
}

pub fn plan_convert(board: &Board, input: &ConvertInput) -> Result<ConvertPlan, String> {
    let label = trimmed(Some(&input.label)).ok_or("label is required")?;
    let kind = match input.into {
        ConvertInto::Project => "project",
        ConvertInto::Release => "release",
    };
    let existing = |reference: &str| match input.into {
        ConvertInto::Project => find_project(board, reference).map(|project| project.id.clone()),
        ConvertInto::Release => find_release(board, reference).map(|release| release.id.clone()),
    };
    let target = match (trimmed(input.id.as_deref()), trimmed(input.name.as_deref())) {
        (Some(reference), _) => Target::Existing(
            existing(&reference).ok_or_else(|| format!("{kind} not found: {reference}"))?,
        ),
        (None, Some(name)) => match existing(&name) {
            Some(id) => Target::Existing(id),
            None => Target::New {
                target_at: match (input.into, &input.target_at) {
                    (ConvertInto::Release, Some(day)) => clean_target(day)?,
                    _ => None,
                },
                description: input
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                name,
            },
        },
        (None, None) => {
            return Err(format!(
                "pass id (an existing {kind}) or name ({kind} to create, or reuse by name)"
            ));
        }
    };
    if let Target::Existing(id) = &target
        && let Some(project) = board
            .projects
            .get(id)
            .filter(|_| input.into == ConvertInto::Project)
        && project.archived_at.is_some()
    {
        return Err(format!(
            "project {} is archived; restore it before filing tickets under it",
            project.name
        ));
    }
    let here = match &target {
        Target::Existing(id) => Some(id.as_str()),
        Target::New { .. } => None,
    };
    let mut carrying: Vec<&Ticket> = board
        .tickets
        .values()
        .filter(|ticket| ticket.labels.iter().any(|l| l.eq_ignore_ascii_case(&label)))
        .collect();
    carrying.sort_by_key(|ticket| (key_number(ticket), ticket.key.clone()));
    let rows = carrying
        .into_iter()
        .map(|ticket| {
            let bucket = match relation(ticket, input.into) {
                _ if ticket.deleted_at.is_some() => Bucket::Deleted,
                None => Bucket::Added,
                Some(current) if Some(current) == here => Bucket::AlreadyHere,
                Some(_) => Bucket::InOther,
            };
            (ticket.id.clone(), bucket)
        })
        .collect();
    Ok(ConvertPlan {
        target,
        label,
        into: input.into,
        rows,
    })
}

#[derive(Debug)]
pub struct Converted {
    pub entity: EntityView,
    pub created: bool,
    /// Ids of the tickets written, by ticket number.
    pub changed: Vec<String>,
}

/// Carry out a plan made on this same board (call both under one lock).
pub fn apply_convert(
    board: &mut Board,
    plan: &ConvertPlan,
    input: &ConvertInput,
    context: &BoardContext,
    at: &str,
) -> Converted {
    let into = plan.into;
    let (entity_id, created) = match &plan.target {
        Target::Existing(id) => (id.clone(), false),
        Target::New {
            name,
            description,
            target_at,
        } => {
            let id = crate::board::new_id();
            match into {
                ConvertInto::Project => {
                    board.projects.insert(
                        id.clone(),
                        Project {
                            id: id.clone(),
                            name: name.clone(),
                            description: description.clone(),
                            lead: None,
                            color: None,
                            source_label: Some(plan.label.clone()),
                            created_at: at.to_string(),
                            updated_at: at.to_string(),
                            archived_at: None,
                        },
                    );
                }
                ConvertInto::Release => {
                    board.releases.insert(
                        id.clone(),
                        Release {
                            id: id.clone(),
                            name: name.clone(),
                            description: description.clone(),
                            target_at: target_at.clone(),
                            released_at: None,
                            source_label: Some(plan.label.clone()),
                            created_at: at.to_string(),
                            updated_at: at.to_string(),
                        },
                    );
                }
            }
            (id, true)
        }
    };
    let (field, entity_name) = match into {
        ConvertInto::Project => ("project", board.projects[&entity_id].name.clone()),
        ConvertInto::Release => ("release", board.releases[&entity_id].name.clone()),
    };
    let mut changed = Vec::new();
    let mut before: HashMap<String, String> = HashMap::new();
    for (id, bucket) in &plan.rows {
        let moves = match bucket {
            Bucket::Added => true,
            Bucket::InOther => input.replace,
            Bucket::AlreadyHere | Bucket::Deleted => false,
        };
        let ends_here = moves || *bucket == Bucket::AlreadyHere;
        if !moves && !(ends_here && input.remove_label) {
            continue;
        }
        let current = relation(&board.tickets[id], into);
        let from = match into {
            ConvertInto::Project => project_name(board, current),
            ConvertInto::Release => release_name(board, current),
        };
        let ticket = board.tickets.get_mut(id).expect("planned ticket exists");
        let mut changes = Vec::new();
        if moves {
            let slot = match into {
                ConvertInto::Project => &mut ticket.project_id,
                ConvertInto::Release => &mut ticket.release_id,
            };
            *slot = Some(entity_id.clone());
            changes.push(Change {
                field: field.into(),
                from,
                to: json!(entity_name),
            });
        }
        if input.remove_label && ends_here {
            let kept: Vec<String> = ticket
                .labels
                .iter()
                .filter(|label| !label.eq_ignore_ascii_case(&plan.label))
                .cloned()
                .collect();
            if kept.len() != ticket.labels.len() {
                changes.push(Change {
                    field: "labels".into(),
                    from: json!(ticket.labels),
                    to: json!(kept),
                });
                ticket.labels = kept;
            }
        }
        if changes.is_empty() {
            continue;
        }
        before.insert(id.clone(), ticket.updated_at.clone());
        ticket.updated_at = at.to_string();
        record(
            ticket,
            "ticket.updated",
            &default_actor(),
            at,
            changes,
            None,
        );
        changed.push(id.clone());
    }
    if input.mark_released_if_done && into == ConvertInto::Release {
        let members: Vec<&Ticket> = live(board, |ticket| {
            ticket.release_id.as_deref() == Some(entity_id.as_str())
        })
        .collect();
        let all_done = !members.is_empty()
            && members
                .iter()
                .all(|ticket| ticket.status == context.done_status);
        // The date the work finished, not the moment of the conversion.
        let finished = members
            .iter()
            .map(|ticket| before.get(&ticket.id).unwrap_or(&ticket.updated_at).clone())
            .max();
        let release = board.releases.get_mut(&entity_id).expect("release exists");
        if all_done && release.released_at.is_none() {
            release.released_at = finished;
            release.updated_at = at.to_string();
        }
    }
    Converted {
        entity: entity_view(board, into, &entity_id, context).expect("entity exists"),
        created,
        changed,
    }
}
