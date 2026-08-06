# Fan-out with a database as the state machine — a worked prompt

The shape this encodes: **the coordinator designs the process, the children are
dumb, and the only way work reports back is through what it writes.** Nothing
here is built into the platform — it is all in the prompt, which is the point.

- Work goes DOWN as direct `harness::spawn` calls. No event starts an agent.
- Each child fetches one file, analyses it, writes one database row, and stops.
  It knows nothing about the other children, the barrier, or the coordinator.
- Work comes UP only as writes. The coordinator learns about them through the
  one binding it registered, gated by `state::barrier` so it wakes **once**
  when every file has landed — not once per file.

## Why a state key AND a database row

The database is the state machine, as asked — but nothing emits database change
events today (`database::row-changed` is not built). So each child writes its
row **and** a one-line marker to state; the coordinator's binding watches the
marker scope. The row is the record; the marker is the doorbell. When the DB
trigger ships, the marker disappears and the binding moves to the table.

## The prompt

Replace the bracketed values. Everything else is literal.

````text
Scan every file in [REPO] for security issues. You are the coordinator: you
design the process, the workers stay dumb, and you never do a file's analysis
yourself.

NAMES — derive these ONCE, now, and use these exact strings everywhere. Never
paraphrase them, never invent a second name for the same thing:
  run id       = scan-[SHORT_RANDOM]
  table        = scan_[SHORT_RANDOM]
  state scope  = scan-[SHORT_RANDOM]
  file key     = the file's path with every non-alphanumeric character replaced
                 by an underscore

SETUP — do all of this in THIS turn, then stop:

1. Create the table, exactly once, with exactly this schema:
   database::execute { "db": "primary", "sql":
     "CREATE TABLE IF NOT EXISTS <table> (file TEXT PRIMARY KEY, status TEXT,
      findings INTEGER, detail TEXT, updated_at TEXT)" }

2. Enumerate the files to scan and count them. That count is N, and the exact
   list of file keys is your expected set. Write the work list to state
   <scope>/_worklist so a later turn can resume it.

3. Register ONE wake, gated so it fires a single time when every file has
   landed:
   engine::register_trigger {
     "trigger_type": "state",
     "config": { "scope": "<scope>" },
     "once": true,
     "conditions": [ { "function_id": "state::barrier",
                       "config": { "id": "<run id>",
                                   "expect": [ <every file key> ],
                                   "carry": "/new_value" } } ]
   }
   `expect` is the EXPLICIT list, never a bare count: a named set tells you
   which file never arrived, a count only tells you that one didn't.

4. Spawn one child per file, in as few messages as possible. Each child task is
   SELF-CONTAINED — it names the file, the table, the scope, and the key, and
   carries no context about the run, its siblings, or you:

   harness::spawn {
     "task": "Read the file at <absolute path>. Analyse it for security issues
              (injection, secrets in source, unsafe eval/exec, missing authz,
              unvalidated input). Then do exactly two writes, IN THIS ORDER,
              and stop — the row first, the marker last:
              1. database::execute INSERT OR REPLACE INTO <table>
                 (file, status, findings, detail, updated_at) VALUES
                 ('<file key>', 'done', <count>, '<one-line summary>',
                  CURRENT_TIMESTAMP)
              2. state::set scope '<scope>' key '<file key>' value
                 { \"file\": \"<file key>\", \"status\": \"done\",
                   \"findings\": <count> }
              The marker in step 2 means "my row is already in the table", so
              it MUST be written after the row and only if the row succeeded.
              If you cannot read or analyse the file, do the SAME two writes in
              the same order with status 'error' and the reason in detail.
              Write them either way — something is waiting on that key. Do
              not create tables, do
              not read other files, do not spawn anything, do not register
              triggers.",
     "session_id": "<run id>-w<i>",
     "options": { "filesystem_root": "<absolute repo root>",
                  "functions": { "allow": ["coder::read-file",
                                           "database::execute",
                                           "state::set"] } }
   }

   Two things about that `options` block are load-bearing, and both fail the
   same way — every child writes an error row saying it could not read its
   file, while the run itself completes cleanly:

   * **You must already hold every function you hand down.** A spawn is
     intersected with YOUR policy and never escalates, so a coordinator without
     `coder::read-file` cannot give it to a child that asks for it. The
     coordinator's own allow list has to be a superset of every child's.
   * **`filesystem_root` is not optional.** `coder::*` runs under a filesystem
     scope, and a child inherits one only from a parent turn that has one.

   If N exceeds what one turn may spawn, spawn the first wave, record a cursor
   at <scope>/_cursor, and arm ONE recurring cron to bring you back for the
   next wave. Drop that cron the moment the cursor is exhausted.

5. End the turn: say what you wired and how many children you started. Do not
   poll. Do not wait.

FINISH — when the wake arrives, it carries every file's summary:

6. Verify against the table, not against the notification:
   database::query { "db": "primary", "sql":
     "SELECT COUNT(*) AS rows, SUM(findings) AS findings,
             SUM(status = 'error') AS errors FROM <table>" }
   `rows` must equal N. If it does not, name the missing file keys (the
   notification's results object has the ones that did arrive), respawn ONLY
   those, re-arm a fresh barrier for the missing set, and stop.

7. Report: files scanned, total findings, per-file lines for anything with
   findings > 0 or status 'error', and the table name so the rows can be read
   directly. Unregister anything still armed. Stop.

RULES for every child, restated because they are what keeps this one-way:
- one file, one analysis, two writes, stop
- never CREATE TABLE — the schema exists before any child runs
- never spawn, never register a trigger, never poll
- on failure, still write both records with status 'error'
````

## Seen in the live run

Three of six children wrote their state marker and never wrote their row. The
barrier completed on six markers, the coordinator woke, queried the table,
found `rows: 3` against `N = 6`, and named the three missing files instead of
reporting success — which is the whole reason step 6 reads the table and not
the notification. The ordering rule above is the fix for the children; the
verification is the safety net for when they still get it wrong.

## What this avoids

Each rule is a failure someone has already hit:

| Rule | What it prevents |
|---|---|
| Names derived once, used verbatim | A gate counting `results_v2` while children write `results`; a run that never terminates |
| Children never `CREATE TABLE` | A child that hits "no such table", creates its own unprefixed one, and writes there — the coordinator then counts an empty table forever |
| `expect` is an explicit key list | A barrier that starves at N−1 with no way to know which producer never came |
| Write the marker on error too | One failed file stranding the whole run |
| Row first, marker last | A marker that means "done" while the row is missing: the coordinator wakes to a table short of N and cannot tell stalled from lost |
| Verify from the table, not the notification | Trusting the message instead of the record |
| Coordinator holds every function it hands down | A child granted `coder::read-file` by a coordinator that lacks it — silently intersected away |
| `filesystem_root` on every child | Children that cannot read the file they were sent to analyse |
| One barrier-gated wake | N paid wake-turns to learn one fact |
