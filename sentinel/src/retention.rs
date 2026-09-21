//! What the store keeps, and what it lets go.
//!
//! The counters on a group never shrink — an incident's size is part of what
//! it means — but the frozen bundles behind it do. What is kept is chosen to
//! answer the question somebody will ask months later: the first occurrence,
//! the most recent ones, and **one per distinct worker version**, because
//! comparing the failure before and after a release is the whole point of
//! having kept anything at all.

use serde_json::{json, Value};

use crate::store::{Db, Store};
use crate::{ids, SentinelError, WorkerConfig};

const DAY_MS: i64 = 86_400_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneOutcome {
    pub buckets_removed: u64,
    pub groups_archived: u64,
    pub evidence_pruned: u64,
}

/// The daily pass: old buckets, long-quiet resolved groups, and the evidence
/// of groups that have accumulated more than they are allowed to keep.
pub async fn prune<D: Db>(
    store: &Store<D>,
    config: &WorkerConfig,
) -> Result<PruneOutcome, SentinelError> {
    let now = ids::now_ms();

    let buckets_removed = store
        .db()
        .execute(
            "DELETE FROM sentinel_buckets WHERE hour_ms < ?",
            vec![json!(now - config.retention.buckets_days as i64 * DAY_MS)],
        )
        .await?;

    // An archived group keeps its row and its counts; what goes is the
    // evidence, which is what takes the space.
    let archive_before = now - config.retention.resolved_ttl_days as i64 * DAY_MS;
    let groups_archived = store
        .db()
        .execute(
            "UPDATE sentinel_groups SET archived = 1, updated_ms = ? \
             WHERE archived = 0 AND status = 'resolved' AND last_seen_ms < ?",
            vec![json!(now), json!(archive_before)],
        )
        .await?;

    let mut outcome = PruneOutcome {
        buckets_removed,
        groups_archived,
        evidence_pruned: 0,
    };
    if outcome.groups_archived > 0 {
        outcome.evidence_pruned += store
            .db()
            .execute(
                "UPDATE sentinel_occurrences SET evidence = NULL, evidence_bytes = 0 \
                 WHERE evidence IS NOT NULL AND group_id IN \
                 (SELECT id FROM sentinel_groups WHERE archived = 1)",
                vec![],
            )
            .await?;
    }

    for group_id in groups_over_budget(store, config).await? {
        outcome.evidence_pruned += prune_group(store, config, &group_id).await?;
    }
    Ok(outcome)
}

/// Prune one group, cheaply, on the ingest's own path.
pub async fn prune_group<D: Db>(
    store: &Store<D>,
    config: &WorkerConfig,
    group_id: &str,
) -> Result<u64, SentinelError> {
    let ignored = store
        .group_by_id(group_id)
        .await?
        .map(|group| group.state.status == crate::GroupStatusV1::Ignored)
        .unwrap_or(false);
    // An ignored group is one somebody asked not to hear about: it keeps the
    // latest bundle and nothing else.
    let keep_recent = if ignored {
        1
    } else {
        config.retention.evidence_per_group as i64
    };

    let dropped = store
        .db()
        .execute(
            "UPDATE sentinel_occurrences SET evidence = NULL, evidence_bytes = 0 \
             WHERE group_id = ? AND evidence IS NOT NULL AND id NOT IN ( \
               SELECT id FROM sentinel_occurrences WHERE group_id = ? \
               ORDER BY at_ms ASC LIMIT 1 \
             ) AND id NOT IN ( \
               SELECT id FROM sentinel_occurrences WHERE group_id = ? \
               ORDER BY at_ms DESC LIMIT ? \
             ) AND id NOT IN ( \
               SELECT MIN(id) FROM sentinel_occurrences WHERE group_id = ? \
                 AND worker_version IS NOT NULL GROUP BY worker_version \
             )",
            vec![
                json!(group_id),
                json!(group_id),
                json!(group_id),
                json!(keep_recent),
                json!(group_id),
            ],
        )
        .await?;

    // Beyond the row cap the oldest rows go entirely, except the first: the
    // group's own beginning is worth more than any later sample.
    store
        .db()
        .execute(
            "DELETE FROM sentinel_occurrences WHERE group_id = ? AND id NOT IN ( \
               SELECT id FROM sentinel_occurrences WHERE group_id = ? \
               ORDER BY at_ms ASC LIMIT 1 \
             ) AND id NOT IN ( \
               SELECT id FROM sentinel_occurrences WHERE group_id = ? \
               ORDER BY at_ms DESC LIMIT ? \
             )",
            vec![
                json!(group_id),
                json!(group_id),
                json!(group_id),
                json!(config.retention.occurrences_per_group as i64),
            ],
        )
        .await?;
    Ok(dropped)
}

/// Groups holding more bundles than their budget allows.
async fn groups_over_budget<D: Db>(
    store: &Store<D>,
    config: &WorkerConfig,
) -> Result<Vec<String>, SentinelError> {
    let rows = store
        .db()
        .query(
            "SELECT group_id, COUNT(*) AS kept FROM sentinel_occurrences \
             WHERE evidence IS NOT NULL AND group_id IS NOT NULL \
             GROUP BY group_id HAVING kept > ?",
            // The first occurrence and one per version sit on top of the
            // recent ones, so this is a floor rather than the exact budget.
            vec![json!(config.retention.evidence_per_group as i64 + 1)],
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            row.get("group_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}
