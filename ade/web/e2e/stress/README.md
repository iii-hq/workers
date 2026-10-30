# Traces stress test

Measures how the traces page behaves on very large traces, the way a busy
engine produces them, and fails when it regresses. Runs with no engine:
the page at `/stress.html` mounts the real `TracesV2` surface over a
generated trace (`src/pages/TracesV2/fixtures/stress-traces.ts`) served by
a fake client (`src/stress/stress-sdk.ts`) that can also stream it.

```sh
pnpm test:stress                          # 500, 2000 and 5000 spans, budgets enforced
STRESS_SIZES=10000 pnpm test:stress       # one size
STRESS_BASELINE=1 pnpm test:stress        # record only, no assertions
STRESS_RELAX=2 pnpm test:stress           # double every budget (slow CI box)
CONSOLE_E2E_BROWSER_CHANNEL=chrome pnpm test:stress   # when the bundled Chromium is missing
```

The spec starts its own dev server on port 4190 (reused if already up),
drives four phases per size and reads the figures off the Chrome DevTools
protocol:

| phase | what happens | what is read |
| --- | --- | --- |
| load | open the page, wait for the timeline, GC | DOM nodes in the traces section, bars in the DOM, heap |
| stream | advance the live stream through 10 ticks | main-thread ms per tick (script + style + layout), longest task, heap after GC |
| hover | sweep the pointer across the timeline | main-thread ms for the sweep |
| waterfall | switch views and scroll the virtualised list | rows in the DOM, main-thread ms |

Every run writes `target/console-stress/report.json` and prints one table.
The budgets live in `budgets()` at the top of `traces-stress.spec.ts`; each
one scales with the span count and was set from a measured run with
headroom, so a change that doubles the work fails here first.

To look at a size by hand: `pnpm dev`, then open
`http://localhost:5173/stress.html?spans=5000&stream=1&theme=dark` and call
`window.__stress.advance(0.8)` in the console to move the stream.
