# examples/catalog2

The catalogue 2 pieces the kanban app uses, one page each, for a host's
instrumented tests: `Markdown`, `Lanes`, `List` drag and drop,
`ListDetail.listFill` and the named hues. It declares `catalog: 2`, calls no
engine function (a host loads it with a fake engine), and its namespace is
`catalog2`.

```bash
pnpm -F @iii-dev/native-ui build:catalog2   # → examples/catalog2/dist/catalog2.js (git-ignored)
```

The ide copies the file to
`android/app/src/androidTest/assets/workerui/catalog2.js`. Its tests in the
harness: `test/example.test.ts`.

## testTags

Every one below is a node's `testTag`; the ones bound to an item field
(`row.tag`) give each row its own tag. In a pager (compact), only the lane
in view is drawn.

### Root

| Tag | What |
|---|---|
| `open-markdown`, `open-markdownLong`, `open-lanes`, `open-dnd`, `open-fill`, `open-hue` | Buttons that push each page |

### `markdown`

| Tag | What |
|---|---|
| `md-doc` | Markdown, selectable, with `onLink`: three headings, a bullet list (one nested item), a numbered list, a `ts` code block, a table (Key/Status/Hue), the links `https://example.com` and `kan:KAN-12`, the bare `https://iii.dev`, a quote, an image (alt text `An image`) and raw HTML |
| `md-last-link` | Text: `none`, then the href of the last link tapped in `md-doc` or `md-dense` |
| `md-dense` | Markdown, dense, with `onLink`: a link to `https://example.com/dense` |
| `md-nolink` | Markdown without `onLink`: a tap on its link (`https://example.com/nothing`) does nothing |

### `markdownLong`

| Tag | What |
|---|---|
| `md-long` | Markdown longer than `limits.maxMarkdownChars` (200 000): `# Long text`, an early link `https://example.com/early`, numbered paragraphs, and a link past the cut (`https://example.com/past`), which the host never draws |
| `md-long-length` | Text: the text's length in characters |
| `md-long-last-link` | Text: `none`, then the last href tapped |

### `lanes`

| Tag | What |
|---|---|
| `lanes` | Lanes: titles Backlog, Todo, Doing, Review, Done; counts `30` each; `onSelect` sets `selected` |
| `lanes-selected` | Text: the selected lane's index (`0` at first) |
| `lanes-list-0` … `lanes-list-4` | One List per lane, 30 items each (keys `<lane>-<n>`, n from 0) |
| `lanes-item-<lane>-<n>` | A ListRow: title `<Lane title> <n + 1>`, subtitle `Lane <lane>, item <n>` |

### `dnd`

| Tag | What |
|---|---|
| `dnd-lanes` | Lanes: Todo, Doing, Done, with live counts (`5`, `3`, `2` at first) |
| `dnd-list-0` … `dnd-list-2` | Lists in the drag group `cards`; Todo has c1–c5, Doing c6–c8, Done c9–c10 |
| `dnd-card-c<n>` | A Card with `press` and `longPress`: a StatusDot and a Badge in the card's hue, the title `Card <n>` |
| `dnd-last` | Text: `none`, then the last drop as `key:index:sourceLane` (`c2:0:0`) |
| `dnd-pressed` | Text: `none`, then the key of the last card pressed |
| `dnd-longpress` | Text: `none`, then the key of the last card long-pressed (released without moving) |
| `dnd-reorder` | A List outside Lanes, 220 dp high, in the drag group `order` (cards cannot land on it) |
| `order-s1` … `order-s6` | Its rows, `Step 1` … `Step 6` |
| `dnd-order` | Text: its keys in order, `s1,s2,s3,s4,s5,s6` at first |
| `dnd-order-last` | Text: `none`, then its last drop as `key:index` |

Each drop handler splices both lists in one batch, as a real app does.

### `fill`

| Tag | What |
|---|---|
| `fill` | ListDetail with `listFill`: the board takes the whole width until a card opens |
| `fill-lanes` | Lanes: Todo, Doing, Done (same cards as `dnd`, no drag group) |
| `fill-selected` | Text: the selected lane's index |
| `fill-list-0` … `fill-list-2` | The lanes' Lists |
| `fill-card-c<n>` | A Card; a tap opens it: `selected` becomes its lane and `showDetail` true, in one message |
| `fill-detail-id` | Text in the detail: the card's key |
| `fill-detail-body` | Markdown in the detail, with links `https://example.com/c<n>` and `kan:KAN-<n>`; a tap toasts the href |
| `fill-close` | Button in the detail page's actions: closes the card |

### `hue`

| Tag | What |
|---|---|
| `hue-<hue>` | A Row per hue (`none`, `blue`, `purple`, `teal`, `green`, `amber`, `rose`) |
| `hue-icon-<hue>`, `hue-badge-<hue>`, `hue-dot-<hue>`, `hue-chip-<hue>` | An Icon (`circle`, 20 dp), a Badge and a Chip (the hue's name as text) and a StatusDot in that hue |
| `hue-list` | A List whose template binds the hue to the item's (`hue={row.hue}`, the name in the script, the index on the wire) |
| `hue-item-<hue>` | Its rows: a StatusDot and a Badge (text = the hue's name) |
