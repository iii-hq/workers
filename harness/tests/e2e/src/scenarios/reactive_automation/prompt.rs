use super::names::ScenarioNames;

pub(super) fn build(names: &ScenarioNames, watchdog_seconds: u64) -> String {
    format!(
        r#"Validate trigger-based orchestration using the existing run id `{run_label}`.

If database capability is not currently available, discover and install the database worker from
the registry before proceeding, then confirm its functions are registered.

Use database `primary`, SQL-safe namespace `{namespace}`, and these exact resources:

- orders: `{orders}` (`id`, `writer`, `amount`, `created_at`)
- writer status: `{writers}` (`writer`, `status`)
- aggregates: `{totals}` (`writer`, `order_count`, `amount_sum`)
- report: `{report}`
- writer sessions: `{writer_1}`, `{writer_2}`, `{writer_3}`
- finalizer session: `{finalizer}`

Namespace every other resource and session with `{run_label}`.

Required execution:

1. Before spawning writers, list the available trigger types and probe
   `database::row-changed` against `{orders}`. Allow at most 30 seconds for the probe, remove its
   test row afterward, and record the result.

2. Before spawning writers, register a namespaced `state` trigger targeting `harness::react`.
   Use it as the notification path if the database trigger is unsupported or does not fire.

3. Spawn the three writer sessions in parallel. Each writer must insert exactly five unique
   orders, one at a time and about two seconds apart; use its matching name (`writer-1`,
   `writer-2`, or `writer-3`), a numeric amount, and a non-null `created_at`; emit the state
   signal immediately after each insert when using the fallback; then mark itself `done` in
   `{writers}`.

4. Each notification must wake a trigger-spawned reactor session namespaced with `{run_label}`.
   Reactors must recompute and upsert `{totals}` from `{orders}` so replaying an event cannot
   double-count. Use per-writer upserts; do not delete all totals before reinserting them, because
   overlapping notifications would expose a transient empty or partial aggregate table.

5. Before spawning writers, arm a namespaced completion reaction covering all three exact writer
   sessions. It must evaluate finalization only after every writer has marked itself `done`; do
   not rely only on an order-insert notification, because the final insert happens before that
   writer's status update. Once all writers are done and totals cover 15 orders, a
   trigger-spawned finalizer in `{finalizer}` must write exactly one row to `{report}` with:

   `run_id`, `watch_mechanism`, `fallback_reason`, `events_received`, `rows_written`,
   `elapsed_ms`, `totals_match`, `no_notification_loss`, `no_double_counting`,
   `reactor_session_id`, `spawning_event`, `trigger_spawned_reactor`,
   `no_inline_waiting`, and `finalizer_session_id`.

   Use `{run_label}` as `run_id` and `{finalizer}` as `finalizer_session_id`. Report the actual
   watch mechanism, a non-empty fallback reason when fallback is used, 15 events and rows,
   positive elapsed milliseconds, and one trigger-spawned reactor session and spawning event.
   Record a numeric start time before the writers run so the finalizer can compute `elapsed_ms`;
   if exact clock arithmetic is unavailable, use a conservative positive elapsed estimate rather
   than zero. Before signaling completion, query the single report row and verify
   `events_received = 15`, `rows_written = 15`, and `elapsed_ms > 0`; correct the row if any of
   those checks fails. Also verify that `totals_match`, `no_notification_loss`,
   `no_double_counting`, `trigger_spawned_reactor`, and `no_inline_waiting` are all true. The
   finalizer must recompute and upsert the complete 15-order totals before this comparison so it
   does not record a transient reactor result.

6. After the trigger-spawned finalizer writes the report, wake the existing root session
   explicitly. Set the wake reaction's `metadata.session_id` to the current root session id;
   setting only `parent_session_id` spawns a new unnamed child and does not wake the root. Set
   that reaction's `metadata.options.functions.allow` to `["*"]`; a narrower policy can prevent
   the resumed root from listing and unregistering triggers. In the resumed root turn, unregister
   every trigger and subscription created for this run, then list the registered triggers and
   verify that none contains `{run_label}` or `{namespace}`. Do not give the final response before
   the root has resumed and this cleanup check has passed.

This is a deliberately large execution. The harness already applies a {watchdog_seconds}-second
stuck-execution watchdog, so do not register a cron or deadline watchdog for this run. Do not
write the report from the root as a substitute for the trigger-spawned finalizer: wait for
`{finalizer}` to run and write it.

Success requires exact equality between `{totals}` and a direct `GROUP BY` over `{orders}`, no
lost or duplicated orders, trigger provenance for reactor and finalizer sessions, no inline
sleeping or polling in place of triggers, and no remaining run triggers.

Report progress briefly and keep the final response factual."#,
        run_label = names.run_label,
        namespace = names.table_prefix,
        orders = names.orders,
        writers = names.writers,
        totals = names.totals,
        report = names.report,
        writer_1 = names.writer_sessions[0],
        writer_2 = names.writer_sessions[1],
        writer_3 = names.writer_sessions[2],
        finalizer = names.finalizer_session,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_wake_preserves_cleanup_access_without_a_competing_watchdog() {
        let prompt = build(&ScenarioNames::new("abcd-rest"), 600);

        assert!(prompt.contains("reaction's `metadata.options.functions.allow` to `[\"*\"]`"));
        assert!(prompt.contains("harness already applies a 600-second\nstuck-execution watchdog"));
        assert!(prompt.contains("do not register a cron or deadline watchdog for this run"));
    }
}
