# examples/catalog3

The catalogue 3 pieces, for a host's instrumented tests and screenshots: an
overview laid out like a dashboard, then one page per piece (`Checkbox`,
`Avatar` and `Timestamp`, `Card.tone`, `Chart` and `Stat`, `Gauge` and
`Meter`, `Steps`, `CodeView`, `Waveform`, and a chat log with `List.anchor`
and a toast with an action). It declares `catalog: 3`, calls no engine
function (a host loads it with a fake engine), and its namespace is
`catalog3`.

```bash
pnpm -F @iii-dev/native-ui build:catalog3   # → examples/catalog3/dist/catalog3.js (git-ignored)
```

The ide copies the file to
`android/app/src/androidTest/assets/workerui/catalog3.js` and
`apple/IiiIdeUITests/Fixtures/workerui/catalog3.js`. Its tests in the
harness: `test/example.test.ts`.

## testTags

### Root

| Tag | What |
|---|---|
| `open-overview`, `open-checkbox`, `open-people`, `open-cards`, `open-charts`, `open-gauges`, `open-steps`, `open-code`, `open-wave`, `open-chat` | Buttons that push each page |

### `overview`

| Tag | What |
|---|---|
| `ov-stats` | Row of Stats: `ov-agents` (6, bot, purple, a detail), `ov-prs` (3, git-pull-request, teal, delta +1 success), `ov-cost` ($4.20 with a sparkline; only on a wide window) |
| `ov-question` | Card, tone attention: an Avatar (Claude, bot, purple, status warn), a question, a short Timestamp, two Buttons |
| `ov-answer` | Text: `Answered: side by side` or `stacked` after a button; hidden before |
| `ov-latency` | Chart, area, two series (`api`, `db`), 20 hourly labels, a threshold at 250 (SLO, danger), axes, unit ms, height 160 |
| `ov-cpu`, `ov-mem`, `ov-budget` | Gauges: 0.42; 0.81 with thresholds 0.8/0.95 (warning); half, 0.64, text `$64`, label `of $100`, amber, size 96 |
| `ov-disk` | Meter 38/21/12 of 100, labels Worktrees, Logs, Cache, legend |
| `ov-steps` | Steps, horizontal: Contract (done), Hosts (active, 0.6), Web port, Docs |

### `checkbox`

| Tag | What |
|---|---|
| `cb-all` | Checkbox All: checked when the four are, indeterminate when some are; checking it checks them all |
| `cb-0` … `cb-3` | Checkboxes with strike: Write the contract and Android host checked at first |
| `cb-count` | Text: `2 of 4 done` at first |
| `cb-disabled` | Checked, disabled |
| `cb-stubborn` | The app keeps it unchecked: a tap goes back |

### `people`

| Tag | What |
|---|---|
| `av-name-0` … `av-name-5` | Avatars with initials, colour from the name (Ada Lovelace, Grace Hopper, Linus, Margaret Hamilton, Alan Turing, Barbara Liskov) |
| `av-image`, `av-icon`, `av-rounded`, `av-empty` | A data URI image; bot in purple; square-terminal in green, rounded; no name (neutral) |
| `av-s20`, `av-ok`, `av-warn`, `av-alert` | Sizes 20, 32, 44, 56 with statuses |
| `ts-relative`, `ts-relative-days`, `ts-future`, `ts-short`, `ts-elapsed` | Timestamps 5 min ago, 3 days ago, in 2 hours, short 3 h, elapsed from 65 s ago (relative to when the page opened) |
| `ts-date`, `ts-time`, `ts-datetime` | 27 Nov 2026 14:05 UTC as a date, a time and both |

### `cards`

`card-default`, `card-attention`, `card-success`, `card-warning`, `card-danger`, `card-accent`, `card-muted`: a Card per tone.

### `charts`

| Tag | What |
|---|---|
| `chart-picked` | Text: `none`, then the last pick (`line 0 04:00`, `bar 4 Fri 30`, `stat 19 228`) |
| `chart-line` | Line, two series, legend, labels, threshold 250, axes, ms; onSelect |
| `chart-bar` | Bars, one series (purple), Mon–Sun, axes; onSelect |
| `chart-bars2` | Bars, two series side by side (blue, rose), legend |
| `chart-gap` | Area with two null points, no axes, green, height 64 |
| `stat-p95`, `stat-errors`, `stat-plain` | Stats: p95 182 ms with a 20-point sparkline and onSelect; Errors 27 with bars; Workers 14 with a detail and no series |

### `gauges`

| Tag | What |
|---|---|
| `gauge-ok`, `gauge-warn`, `gauge-danger` | Rings at 0.35, 0.85, 0.97 with thresholds 0.8/0.95 |
| `gauge-half` | Half, teal, size 120, value from 0.5, changed by `gauge-less` and `gauge-more` (±10 %) |
| `gauge-text` | Ring 0.66 with text `2/3`, purple |
| `meter-disk`, `meter-split` | Meters: 38/21/12 of 100 with a legend; 5/3/2 green/amber/rose, thickness 14, legend |

### `steps`

| Tag | What |
|---|---|
| `steps-picked` | Text: `none`, then `<index> <title>` of the step tapped in `steps-vertical` |
| `steps-vertical` | Seven steps, every state (done, done, active 0.7, blocked, failed, skipped, pending), details |
| `steps-horizontal` | Four milestones |
| `steps-compact` | The seven, horizontal and compact |

### `code`

| Tag | What |
|---|---|
| `code-picked` | Text: `none`, then `line <n>` |
| `code-ts` | TSX, line numbers from 40, lines 43–45 marked, onSelect |
| `code-diff` | A diff with a hunk header |
| `code-rust`, `code-wrap` | Rust with a long line: no wrap (scrolls sideways) with numbers; wrap and selectable |
| `code-long` | Plain text longer than `limits.maxCodeChars`, height 160 |

### `wave`

| Tag | What |
|---|---|
| `wave-voice` | 240 peaks, progress 0.3, a marker at 0.6, height 56; a tap seeks (onSelect sets progress) |
| `wave-progress` | Text: `30%`, then the seek as a percent |
| `wave-blue` … `wave-rose` | One per hue, no progress, height 28 |

### `chat`

| Tag | What |
|---|---|
| `chat-log` | List, anchor bottom: messages 71–100 (keys are the ids), each Row tagged `Message <id>` with an Avatar, the name, a short Timestamp and the text; onStartReached loads 20 older, down to 1 |
| `chat-loads` | Badge: `<n> loads` |
| `chat-state` | Text: `deleted <id>`, then `restored <id>` after Undo |
| `chat-delete` | Removes the last message and shows a toast with Undo; Undo puts it back |
| `chat-send` | Appends a message (101, 102, …) |
