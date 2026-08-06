use super::names::ScenarioNames;

pub(super) fn build(names: &ScenarioNames, watchdog_seconds: u64) -> String {
    format!(
        r#"Validate parent-owned reactive orchestration using the existing run id `{run_label}`.

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

Architecture rules:

- A trigger binding never starts an agent. It either wakes its owner when `function_id` is omitted,
  or calls one plain function without a model turn.
- Spawn writers and the finalizer directly with `harness::spawn`.
- Do not target any `harness::*` function from a binding, and do not poll.
- The Harness already applies a {watchdog_seconds}-second stuck-execution watchdog. Do not add a
  timer or cron.

Phase 1 — bounded database-trigger probe:

1. List the available trigger types and create the four tables. Use these columns:
   - `{orders}`: `id TEXT PRIMARY KEY, writer TEXT, amount REAL, created_at TEXT`
   - `{writers}`: `writer TEXT PRIMARY KEY, status TEXT`
   - `{totals}`: `writer TEXT PRIMARY KEY, order_count INTEGER, amount_sum REAL`
   - `{report}`: `run_id, watch_mechanism, fallback_reason, events_received, rows_written,
     elapsed_ms, totals_match, no_notification_loss, no_double_counting,
     reaction_function_id, reaction_event, mechanical_reaction, no_inline_waiting,
     finalizer_session_id`

2. Register a one-shot wake-only `database::row-changed` binding on `{orders}`, label it
   `{run_label}-probe`, insert one probe row, and END YOUR TURN. Do nothing else until that database
   wake arrives. The scenario watchdog is the bounded failure path if the trigger is broken.

Phase 2 — arm reactions before fan-out:

3. When the probe wake arrives, delete the probe row. Register a standing call binding, label
   `{run_label}-aggregate`, on INSERT events for `{orders}`. Its target must be
   `database::execute` with `db: "primary"` and this idempotent aggregate SQL:

   `INSERT OR REPLACE INTO {totals} (writer, order_count, amount_sum)
    SELECT writer, COUNT(*), SUM(amount) FROM {orders}
    WHERE writer IN ('writer-1','writer-2','writer-3') GROUP BY writer`

   Use a call-binding `lifecycle.max_fires` of 15. The event may be injected into an unused payload
   field; `database::execute` ignores unknown fields. This reaction is deterministic and must not
   wake any model session.

4. Register one wake-only `state` binding for scope `{run_label}`, key `writer_done`, label
   `{run_label}-writers-complete`, `once: true`, with:

   `conditions: [{{ function_id: "state::barrier", config: {{
     id: "{run_label}-writers", expect: ["writer-1","writer-2","writer-3"],
     key_from: "/new_value/writer", carry: "/new_value"
   }} }}]`

5. Spawn `{writer_1}`, `{writer_2}`, and `{writer_3}` together in ONE assistant message. Give each
   only `database::execute` and `state::set`. Writer N must:
   - insert five separate rows into `{orders}` with ids `writer-N-1` through `writer-N-5`,
     writer `writer-N`, amounts `N*10+1` through `N*10+5`, and `CURRENT_TIMESTAMP`;
   - after all five inserts, insert or replace `(writer-N, 'done')` in `{writers}`;
   - only after that database status write succeeds, call `state::set` with scope `{run_label}`,
     key `writer_done`, and value `{{"writer":"writer-N"}}`;
   - stop without reads, sleeps, retries over time, trigger registration, or further delegation.

6. After the three spawn calls, END YOUR TURN. Do not query status, orders, totals, sessions, or
   children. Your next activity must come from the barrier wake.

Phase 3 — barrier wake and direct finalization:

7. When `{run_label}-writers-complete` wakes this root session, first register a one-shot wake-only
   `database::row-changed` binding on INSERT into `{report}`, label `{run_label}-report-ready`.
   Then directly spawn exactly one finalizer in `{finalizer}` with only `database::query` and
   `database::execute`. The finalizer must:
   - recompute `{totals}` with the same idempotent SQL above;
   - verify exactly 15 unique orders, three done writers, and exact equality between `{totals}` and
     a direct `GROUP BY` over `{orders}`;
   - write exactly one row to `{report}` itself—never ask the root to write it—with
     `run_id = '{run_label}'`, `watch_mechanism = 'database::row-changed'`,
     a non-empty `fallback_reason` saying no fallback was needed because the probe fired,
     `events_received = 15`, `rows_written = 15`, positive `elapsed_ms`, all integrity booleans
     true, `reaction_function_id = 'database::execute'`,
     `reaction_event = 'database::row-changed insert'`, `mechanical_reaction = true`,
     `no_inline_waiting = true`, and `finalizer_session_id = '{finalizer}'`;
   - stop immediately after the report insert.

8. END YOUR TURN immediately after spawning the finalizer. Do not poll for its result.

Phase 4 — report wake and cleanup:

9. Only after `{run_label}-report-ready` wakes this root session, query the four tables and verify
   the report. Unregister any still-active run binding, then list registered triggers and verify
   none contains `{run_label}` or `{namespace}`. Report PASS or FAIL with the three writer totals,
   order count, reaction delivery, finalizer session, and cleanup evidence.

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
    fn prompt_uses_parent_owned_bindings_without_removed_reactions() {
        let prompt = build(&ScenarioNames::new("abcd-rest"), 600);

        assert!(prompt.contains("A trigger binding never starts an agent"));
        assert!(prompt.contains("target must be\n   `database::execute`"));
        assert!(prompt.contains("function_id: \"state::barrier\""));
        assert!(prompt.contains("directly spawn exactly one finalizer"));
        assert!(!prompt.contains("harness::react"));
        assert!(!prompt.contains("trigger-spawned"));
    }
}
