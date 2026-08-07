window.BENCHMARK_DATA = {
  "lastUpdate": 1786068207122,
  "repoUrl": "https://github.com/iii-hq/workers",
  "entries": {
    "Harness Stack Coverage": [
      {
        "commit": {
          "author": {
            "name": "Ytallo",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "12700ae892089d08ec5e30e447ce92407c75e8ec",
          "message": "(MOT-4295) feat(ci): publish harness integration and e2e coverage to the metrics site (#664)\n\n* feat(ci): publish harness integration and e2e coverage to the metrics site\n\nInstrument the harness stack with LLVM source-based coverage in the daily\nE2E cycle and publish browsable HTML reports plus a line-coverage trend\nto the gh-pages metrics site under dev/harness-e2e/coverage/.\n\n- coverage input on _harness-integration.yml and _harness-e2e.yml:\n  instrumented builds (-Cinstrument-coverage with runtime counter\n  relocation), continuous-mode LLVM_PROFILE_FILE so profiles survive the\n  SIGTERM/SIGKILL stack teardown, dedicated rust-cache keys, and\n  coverage report artifacts\n- new e2e coverage job merges per-matrix profraw artifacts against the\n  packaged stack binaries; the integration job reports in-job\n- harness-e2e-daily.yml enables coverage on both suites so the reports\n  ride the workflow_run the benchmark publisher already consumes\n- harness-e2e-benchmark.yml installs the reports with replace-latest\n  semantics, emits a coverage summary manifest, and records a\n  line-coverage time series via github-action-benchmark\n- shared .github/scripts/coverage_report.sh (llvm-profdata merge +\n  llvm-cov show/report/export) reused by CI and the new local\n  'make -C harness integration-coverage' target\n- the integration process supervisor now passes LLVM_PROFILE_FILE\n  through env_clear so instrumented children keep writing profiles\n\n* refactor(benchmark-site): keep coverage summary on the dedicated landing page\n\n* refactor(benchmark-site): progressive-disclosure IA for the execution detail page\n\nThe detail page rendered everything eagerly: ~40 metric tiles and 15 run\naccordions before any interaction, an unbounded failure list, the full\nprompt expanded per run, and a multi-megabyte JSON.stringify of the whole\ndetail at page load. Reorganize it failure-first:\n\n- failure strip becomes a grouped triage: top-5 chips per failing run\n  (count badge + first message), rest behind 'Show all'; the reliability\n  KPI folds into the strip title (5 -> 4 KPIs)\n- deep links now open every collapsed ancestor <details> (hashchange +\n  delegated clicks), so triage chips land expanded on the right run\n- Overview and Configuration collapse into disclosures with one-line\n  digests; scenario cards become one-line accordions (open only when\n  failed or sole scenario) with the metric tiles moved into the body\n- expanded run bodies switch to five lazily-rendered tabs (Evaluation,\n  Usage, Prompt, Sessions, Raw); prompt clamps at 260px with expand;\n  trace/run/page JSON and the download blob serialize on demand\n- content-visibility: auto on scenario and run accordions\n\nNew additive helper HarnessExecutionData.groupRunFailures with unit\ntests; no existing module APIs changed (23/23 site tests pass).",
          "timestamp": "2026-08-01T01:50:37Z",
          "url": "https://github.com/iii-hq/workers/commit/12700ae892089d08ec5e30e447ce92407c75e8ec"
        },
        "date": 1785568936176,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 44.39,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 43.55,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4b056e50720252afcb05ff4f5722149b7c5db1c6",
          "message": "(MOT-3748) fix(harness): preserve registrations across engine reloads (#666)\n\n* (MOT-3748) fix(harness): preserve registrations across engine reloads\n\n* (MOT-3748) fix(harness): align queue dependency with recovery contract\n\n* (MOT-3748) fix(harness): fall back for legacy queue schemas\n\n* (MOT-3748) test(harness): update queue manifest contract\n\n* (MOT-3748) ci(harness): run quickstart after release\n\n* (MOT-3748) ci(harness): smoke mandatory dependencies",
          "timestamp": "2026-08-01T17:50:47Z",
          "url": "https://github.com/iii-hq/workers/commit/4b056e50720252afcb05ff4f5722149b7c5db1c6"
        },
        "date": 1785654868162,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 43.27,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 44.96,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Guilherme de S. Vieira Beira",
            "username": "guibeira",
            "email": "guilherme.vieira.beira@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4b52a11e730597f7e8743e17f4a12d387943946e",
          "message": "fix(ci): build the tag being released, not the dispatch branch (#669)\n\n`actions/checkout` with no `ref` resolves the ref that started the run. On\na `push: tags:` trigger that is the tag, so the release path was correct by\naccident. On a `workflow_dispatch` it is the branch the dispatch was started\nfrom — `alpha-release.yml` dispatches with `--ref main` — while the tag\nreaches the jobs only as metadata: the version parsed from its name and\n`tag_name` on the GitHub Release.\n\nSo a dispatched release compiles one commit and labels the artifacts with\nanother commit's version, silently. `state/v0.21.4-alpha.2` shipped this way:\nthe tag points at a commit pinning `iii-sdk = \"=0.22.0-alpha.3\"`, but the\nbinary was built from main, which pins `=0.21.6`. The published artifact\ncarries the prerelease version number and none of the code it names — it\ndoes not send `namespace` on `engine::workers::register`, so no worker\nbuilt this way can join a namespaced project.\n\nEvery checkout in the release path now takes the ref being released. The\nreusable workflows get an optional `ref` input defaulting to `''`, which is\n`actions/checkout`'s own default, so nothing changes for a tag push. The\ndispatch keeps using `--ref main`: the pipeline definition should come from\nmain, only the source it compiles should follow the tag.",
          "timestamp": "2026-08-02T18:17:00Z",
          "url": "https://github.com/iii-hq/workers/commit/4b52a11e730597f7e8743e17f4a12d387943946e"
        },
        "date": 1785741806188,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 44.8,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 44.88,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "id": "10a168a75dc2b7088d976621bd76ccaa4d146ff1",
          "message": "feat(harness): add discriminative judge-backed scenarios with anchored rubrics\n\nAdd design_tradeoff (contested scaling decision that punishes non-committal\nanswers) and security_triage (subtle real vulnerabilities plus safe decoys\nthat punish invented findings) to break the ceiling effect of the existing\nsubjective scenarios. Anchor every judge-backed criterion description with\nexplicit full/half/zero score bands to reduce judge variance.",
          "timestamp": "2026-08-02T21:00:44Z",
          "url": "https://github.com/iii-hq/workers/commit/10a168a75dc2b7088d976621bd76ccaa4d146ff1"
        },
        "date": 1785768290334,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 43.97,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 44.89,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "id": "83966dc6acd3d53a54c73e73bc5faf1bf8a3511b",
          "message": "fix(dashboard): filter efficiency sparklines to the comparable cohort\n\nThe sparklines summed raw per-scenario averages across every reported\nscenario, so a missing report or a newly added scenario moved the line for\nstructural reasons while the delta chip on the same card honestly compared\nonly the comparable cohort. Sum the same cohort the chip uses and skip\nexecutions that lack any cohort contract instead of fabricating a dip.",
          "timestamp": "2026-08-03T20:11:12Z",
          "url": "https://github.com/iii-hq/workers/commit/83966dc6acd3d53a54c73e73bc5faf1bf8a3511b"
        },
        "date": 1785789739045,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 45.1,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 44.89,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Anderson Leal",
            "username": "andersonleal",
            "email": "andersonofl@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4c0401a0537802e29fb611e5b561fdae815770cf",
          "message": "(MOT-4335) feat(harness): parent-owned control plane and the post-turn validation hook (#687)\n\n* feat(harness): parent-owned control plane and the post-turn validation hook\n\nA registered trigger binding can no longer start an agent, and a turn can no\nlonger complete without passing the validators attached to it.\n\nCONTROL PLANE. Bindings keep two shapes — wake the owning session (once by\ndefault) or call an allowed plain function (standing by default) — and harness\nturn events are not agent-bindable in any shape. `harness::spawn` is refused as\na target at registration, at delivery, and by a startup sweep that retires\nstored spawn-target records loudly with the register-a-wake-and-spawn-directly\nmigration. Spawning is an in-turn call only: children are capability-walled\nleaves (CONTROL_PLANE_DENY) unless the spawn passes `options.orchestrator`,\nstill capped by the parent, and child outcomes flow only through the medium the\ntask names — the `[child-failure]` injection is gone. Removing the target\ndeleted its supporting cast rather than reimplementing it: the three built-in\nreactive gates (self-cause, depth, upstream-failure) and the lineage registry\nare gone, and topology doctrine is stripped from the default, sub-agent, and\neight provider prompts, with a sweep test that bans process prescriptions\nrepo-wide.\n\nPOST-TURN HOOK. A sixth synchronous hook point, `harness::hook::post-turn`,\nattached with an ordinary `engine::register_trigger` and awaited inside\nfinalize, so it gates completion the way `pre_*` hooks gate spend. A deny\nre-prompts the same turn with the validator's own reason (`retry_prompt`\nsupports `{value}`/`{reason}`), bounded by `max_validation_retries` — settable\nper send and per spawn. Two invocation modes: a payload template whose verdict\nis read from the response, or the hook envelope contract. Agents may install\none themselves — the single harness-internal type they may bind — force-scoped\nto their own session or children under their own prefix, with owner-checked\nteardown. An out-of-scope pattern carrying the spawn `-child-` convention is\nstill refused, but names the exact in-scope substitution so the retry is\nmechanical. The `max_turns` cap runs the chain too, so a step-exhausted turn\ncannot finalize unvalidated.\n\nFIXES. A failed delivery claim notifies the owner instead of dropping fires\nsilently; `triggers::list`/`unregister` default `session_id` to the calling\nsession; discovery reports the intercepted registration contract, not the raw\nengine one; trigger bindings claim their fire atomically so two simultaneous\nfires cannot take the same ordinal; a past-deadline unfired wake stays armed\nuntil its expiry notice is delivered; the harness claims its private state\nnamespace at runtime.\n\nCONSOLE. Validation nudges render as machine-authored prompts\n(`⟳ validator · corrective prompt`), recognized by the `validation` origin flag\nor the durable nudge entry id, so a corrective re-prompt is never mistaken for\na user message.\n\nTESTS. 17 e2e quality scenarios, all passing at score 100 on an isolated stack:\neight cover the validation loop (self-installed validator, sub-agent gating\nsingle and multi, a failure path, a custom validator function on a temporary\nworker, LLM-driven self-repair, scope enforcement, chained validators) and\nreactive workflow coverage joins them. The integration suite is rebuilt around\nthe new topology. Two scenario gates that encoded assumptions the stack no\nlonger holds are corrected: `research_pipeline` no longer deletes the reserved\n`state_barrier` scope, and `receiving_operation` no longer treats read-only\ncontract lookup as a privilege violation. CI now builds and packages the fp\nworker that `run-ci.sh` waits on, and the tree is clean under\n`clippy --all-targets -D warnings`.\n\nCOMPATIBILITY. Stored spawn-target bindings never dispatch again (retired\nloudly, owner notified). Agent-bindable turn events are gone in every shape.\nParentless direct spawns are leaf-by-default and children lose `harness::send`\nunless granted. The lineage-based unregister grant is removed. Reactive\nsubscriptions must be re-registered.\n\n* fix(prompts): sync provider identity prompts to harness default, gate them off by default\n\nAll eight provider prompts/identity.txt were stale generations of the\ncompressed variant; overwrite them with harness/prompts/default.txt so\nevery provider serves the surface the harness actually ships.\n\nFlip provider_identity_prompt to default false: turns pin to the\nembedded default prompt unless the operator opts into provider-served\nprompts via router::system_prompt::get.\n\n* fix(harness): confine in-turn spawn reuse to the caller's own tree\n\nModels are not RNGs: two runs of the same prompt re-invent the same\n\"random\" child session id, and harness::spawn silently reused the\nexisting session - old transcript retained, console still nested under\nthe original parent (one live courier session was spawned by three\ndifferent runs across a week). An in-turn spawn naming an EXISTING\nsession now refuses unless the target is the caller itself or a child\nit spawned, naming the current owner and the remedy. Parentless\n(direct) spawns keep today's semantics: forks and reaction delivery\nlegitimately target foreign sessions.\n\nReuse is also visible now: ChildIds and SpawnResponse carry `reused`,\nthe in-turn result appends a reuse note, and the session_id schema\ndescription teaches the constraint. The session client gains\n`metadata_of` (exists() delegates to it).\n\nINT-018 spawn-reuse-guard drives both halves end to end: it plants a\nforeign-owned session with a plain session::ensure call, gates the\ncollision on is_error: true, then re-tasks an own child onto its\nretained transcript. Times out on the unfixed harness, where silent\nreuse answers is_error: false.\n\n* fix(harness): a condition that fails to evaluate notifies the binding's owner\n\nA wake gated by a condition whose CALL errors (a barrier pointed at a\npointer the event does not carry, a non-decision function wired as a\ncondition) starves on every fire; the condition-error skip record lands\nin a transcript the PARKED owner never reads, so the session sleeps\nforever next to a binding that can never deliver. Three live\nreceiving-op runs died exactly this way.\n\nSkip::is_condition_failure classifies condition-error / -policy /\n-approval — a healthy condition answering \"not yet\" and lifecycle skips\nstay silent. record_stop then injects a [notification] into the owner\nsession, once per binding via the stable e_condfail_<binding> entry id\n(session-manager's entry-id idempotence is the dedup) — the same\ndoctrine as the claim-failure and wake-expiry notices. The text names\nthe watch and the reason, says the binding stays armed but starving,\nand teaches the reconcile step: a re-registered binding never fires for\nevents that preceded it, so read the watched state once after re-arming.\n\nINT-019 condition-failure-notice pins it end to end: a controlled\nfunction as the broken condition, one probed row insert, the owner\nwoken by the notice, the skip record still written, and the lifecycle\nunconsumed. Times out on the unfixed harness.\n\n* fix(harness): every spawned child keeps the contract-discovery pair\n\nThe sub-agent contract mandates an engine::functions::list/::info round\nbefore the first call, but parents narrow children to just the work\nfunctions (options.functions.allow: [\"db::x\"]) — so the obedient child\nwas policy-denied its mandatory first step and reported\n`FAILED: engine::functions::list/info is denied by policy`, while\nsiblings that skipped discovery succeeded. When compliance loses and\ndisobedience wins, the contract is broken.\n\nchild_functions now unions CHILD_DISCOVERY_ALLOW into any NON-EMPTY\nchild allow-list, after the leaf wall and before the ask-mode clamp: an\nempty allow (deliberate dispatch-disabled) stays empty, glob coverage\nadds no duplicates, an explicit deny still wins and gets no dead allow\nentry, and the operator's ask-mode baseline keeps the final word. The\ngrant is dispatch-level only — the engine's meta surface is not part of\nthe hydrated registry snapshot, so native toolsets are unchanged.\n\nINT-020 child-discovery-granted drives it end to end: a probe-spawned\nchild under exactly the starving whitelist must complete its mandatory\ndiscovery round (is_error: false) before doing its work. The pre-union\nharness answers that call with a policy denial and the run times out.\n\n* fix(prompts): arm-before-produce, reconcile, deadline, and name-agreement doctrine\n\nFive live receiving-op runs died parked next to bindings that could\nnever fire, each a different judgment slip the identity prompt never\nwarned against. The doctrine added, phrased as mechanism plus\nconsequence (tool guidance, not process topology — the\nprescribed-process lint stays clean):\n\n- a binding only sees the future: arm the watch before starting\n  whatever produces the events (spawns included), and after any\n  (re)registration read the watched state once — an event that landed\n  before it will never fire it. Also stated where spawns teach the\n  destination read, and asked again in the final checklist.\n- ALWAYS set a lifecycle deadline on any wake the run cannot finish\n  without.\n- contract discovery is never lost to narrowing — every child keeps\n  engine::functions::list and ::info (the discovery union), so a\n  whitelist needs only the work functions.\n- the task audit covers shared-medium names: the table, scope, or key a\n  task tells a child to write must be byte-identical to what the\n  bindings watch — a namespaced watch fed by a bare-named task never\n  fires (the fifth run's exact death, also on the final checklist).\n\nAll eight provider identity prompts resynced byte-identical to the\nharness default.\n\n* fix(ci): restore approval-gate's SDK pin, apply rustfmt, refresh provider prompt assertions\n\nThree CI failures, all fallout from the rebase and from hand-written code\nthat never met the repo's gates:\n\n- approval-gate pinned `iii-sdk = \"=0.21.6\"` while main had already moved\n  it to `=0.21.8`. My side of the rebase won that line, and because\n  approval-gate carries a path dependency on harness (which pins\n  `=0.21.8`), the two exact pins collided and cargo could not resolve the\n  graph at all — every approval-gate cargo invocation failed before\n  compiling. Restored to main's `=0.21.8` and refreshed the lock.\n- `cargo fmt --all -- --check` failed on the harness workspace and on\n  approval-gate's new `evaluate.rs`. Applied rustfmt; no logic changed.\n- all eight providers assert a third invariant about their identity\n  prompt, and each pinned wording the synced harness default no longer\n  uses (\"IMPORTANT: NEVER invent function ids\", \"Never invent function\n  ids\", \"## Autonomy and persistence\"). Since every provider now ships\n  the same prompt, they now assert the same live invariant: \"Never use a\n  function id from memory.\" — the same intent, in wording that exists.\n\nVerified with the exact CI commands (fmt, clippy --all-targets\n--all-features -D warnings, test --all-features) on harness,\napproval-gate, and all eight providers.\n\n* fix(prompts): a session id given to an agent is used verbatim\n\nThe identity prompt told every agent, unconditionally, to name a child\n\"a short readable slug plus a few random characters\". Handed an exact\nsession id by its task, an agent therefore still appended entropy:\n`receiving-caf42e56-acme` became `receiving-caf42e56-acme-k3m7`. Every\nconsumer watching the specified id then finds nothing — the same\nname-agreement failure this branch already documents for tables, scopes,\nand keys, but for session ids.\n\nCaught by the judged e2e suite: `receiving_operation` scored 45/90 with\nthree gates red (`minimal_spawns`, `courier_execution`,\n`no_late_root_repair`), all from one cause — the couriers ran under\nrenamed sessions, so the evaluator could not find them where the prompt\nsaid they would be. The gates are right and stay as they are; obedience\nto an explicitly given id IS the requirement.\n\nThe entropy rule now applies only when the agent picks the name itself,\nand its justification is restated for the guard that landed earlier on\nthis branch: reuse inside your own tree is reported as `reused`, and a\nspawn into another owner's session is refused outright.\n\n* fix(harness): grant INT-005 the barrier its wake is gated on\n\nINT-005 armed a `state`-wake gated by a `state::barrier` condition but\nnever granted itself `state::barrier`, so `engine::register_trigger`\nrefused the binding outright: \"a reaction can only call functions you\ncan call yourself\". Nothing was armed, the parked root was never woken,\nand the run died downstream of that — in CI as a collect-phase hang\n(the trace oracle waits for 2 turn ids and only ever sees 1), locally as\na generation-2 match failure on the registration's `is_error`. One cause,\ntwo symptoms, and the harness rule behind it is correct: a binding may\nonly gate itself with functions its registrant could call directly.\n\nThe grant goes in sorted position — the harness renders the narrowed\npolicy prompt line sorted while the DSL joins the allow-list in\ninsertion order, so an unsorted entry breaks the system-prompt hash\nmatch instead.\n\nThe full deterministic suite is now green: 16/16 direct scenarios.\n\n* fix(harness): teach the playground path about the database worker\n\nThis branch added the database worker to the required integration stack\n(INT-014 drives a `database::row-changed` wake through the real worker),\nand the playground driver shares that stack config — but four places\nthat enumerate the playground worker set were never updated. The runner\nexited at boot with `runner_error: missing --worker-bin for: database`,\nno ready manifest was ever written, and every Console Playwright spec\ntimed out identically waiting for it. That cascade is why all four\npre-existing specs (durable-hydration, exactly-once-function,\nmulti-turn-traces, ui-send) went red in CI while all sixteen direct\nscenarios passed: the specs were fine, the stack never booted.\n\nUpdated: the Playwright fixture's workerArgs (`DATABASE_BIN`), the CI\nworkflow env, the `integration-playground` make target, and the README\nexample. Verified end to end: the playground publishes its ready\nmanifest and proceeds to await, and the full Console suite passes\nlocally in 15.5s — 4/4.",
          "timestamp": "2026-08-03T22:43:58Z",
          "url": "https://github.com/iii-hq/workers/commit/4c0401a0537802e29fb611e5b561fdae815770cf"
        },
        "date": 1785830226779,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 44.47,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 45.14,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "workers-ci[bot]",
            "email": "workers-ci[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "workers-ci[bot]",
            "email": "workers-ci[bot]@users.noreply.github.com"
          },
          "id": "a4b7b49463e78b2b588292d416cbbf2e7f23d1d6",
          "message": "chore(fp): bump to v0.2.5",
          "timestamp": "2026-08-05T05:09:56Z",
          "url": "https://github.com/iii-hq/workers/commit/a4b7b49463e78b2b588292d416cbbf2e7f23d1d6"
        },
        "date": 1785917069155,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 44.15,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 45.16,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "id": "450995a8672a43416c09c73c193ab2b035dc8dd7",
          "message": "fix(providers): sync deepseek and zai locks with llm-router 1.4.2\n\nThe standalone provider lockfiles still pinned llm-router 1.4.1 after the\npath dependency was bumped, so every --locked build of provider-deepseek\nand provider-zai fails (Harness E2E Daily has been red since the bump).\nRegenerated with cargo metadata; no other entries changed.",
          "timestamp": "2026-08-06T18:26:15Z",
          "url": "https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7"
        },
        "date": 1786066632081,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 45,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 45.18,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "8c0c1ae621c439a49c097b073a85c1b13f5cc2e8",
          "message": "fix(providers): sync deepseek and zai locks with llm-router 1.4.2 (#731)\n\nThe standalone provider lockfiles still pinned llm-router 1.4.1 after the\npath dependency was bumped, so every --locked build of provider-deepseek\nand provider-zai fails (Harness E2E Daily has been red since the bump).\nRegenerated with cargo metadata; no other entries changed.",
          "timestamp": "2026-08-06T18:33:27Z",
          "url": "https://github.com/iii-hq/workers/commit/8c0c1ae621c439a49c097b073a85c1b13f5cc2e8"
        },
        "date": 1786068109777,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 45.43,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 45.27,
            "unit": "%"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "committer": {
            "name": "Ytallo Layon",
            "username": "ytallo",
            "email": "ytallo.layon@gmail.com"
          },
          "id": "450995a8672a43416c09c73c193ab2b035dc8dd7",
          "message": "fix(providers): sync deepseek and zai locks with llm-router 1.4.2\n\nThe standalone provider lockfiles still pinned llm-router 1.4.1 after the\npath dependency was bumped, so every --locked build of provider-deepseek\nand provider-zai fails (Harness E2E Daily has been red since the bump).\nRegenerated with cargo metadata; no other entries changed.",
          "timestamp": "2026-08-06T18:26:15Z",
          "url": "https://github.com/iii-hq/workers/commit/450995a8672a43416c09c73c193ab2b035dc8dd7"
        },
        "date": 1786068205986,
        "tool": "customBiggerIsBetter",
        "benches": [
          {
            "name": "E2E line coverage",
            "value": 44.67,
            "unit": "%"
          },
          {
            "name": "Integration line coverage",
            "value": 45.18,
            "unit": "%"
          }
        ]
      }
    ]
  }
}