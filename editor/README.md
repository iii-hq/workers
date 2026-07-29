# editor

A line an agent wrote through shell::fs::write, without ever calling editor::*.

A code workspace that an agent and a person share. Open a folder, and the
buffers you have open, the folders you have expanded, and the mtimes each
buffer was read at are one record on the bus — so the file an agent opens
appears in your tabs, and the file you open is one the agent can see.

The unit is a **folder**, not a repository. The tree, the tabs, the editor and
the finder all work in a plain directory; git adds a branch label and change
marks when the root happens to be a repo, and nothing else changes when it
is not.

It opens no files itself. Reads, writes, moves, listings and `git` all go
through the [`shell`](../shell/) worker, so shell's jail and denylist are the
only filesystem boundary; the workspace record lives in [`state`](../state/).
What `editor` adds is the model on top: diffing, ranking paths, refusing a
stale write, and keeping open buffers correct when a folder moves under them.

## Install

```bash
iii worker add editor
iii worker add shell   # required — editor has no filesystem access of its own
iii worker add state   # required — the workspace record lives here
```

### Companion workers

| Worker | Why |
|---|---|
| [`shell`](../shell/) | Required. Every read, write, move, listing (`coder::tree`) and `git` invocation. Its `fs.host_roots` jail governs which paths `editor` can reach. |
| [`state`](../state/) | Required. Holds the active root and one session per project (open buffers, expanded folders). |
| [`console`](../console/) | Optional. Renders the `#/ext/editor` page and the `editor::*` chat cards. |

## Quickstart

```rust
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());
    let call = |id: &str, payload| iii.trigger(TriggerRequest {
        function_id: id.into(), payload, action: None, timeout_ms: Some(30_000),
    });

    // Any folder. No repository required.
    call("editor::workspace::open", json!({ "root": "/srv/app" })).await?;

    let file = call("editor::open", json!({ "path": "src/main.rs" })).await?;
    let edited = file["content"].as_str().unwrap().replace("TODO", "done");

    // Show the change before making it.
    let preview = call("editor::diff", json!({
        "before": file["content"], "after": edited, "path": "src/main.rs",
    })).await?;
    println!("{}", preview["patch"].as_str().unwrap());

    // The mtime from the open is what makes this safe: if anything else
    // touched the file in between, nothing is written and the divergence
    // comes back as a patch.
    let saved = call("editor::save", json!({
        "path": "src/main.rs", "content": edited, "expected_mtime": file["mtime"],
    })).await?;
    if saved["conflict"] == true {
        println!("refused:\n{}", saved["conflict_patch"].as_str().unwrap());
    }
    Ok(())
}
```

## Functions

| Function | Does |
|---|---|
| `editor::workspace::open` | Point the workspace at a folder. Returns the buffers and expanded folders remembered for it. |
| `editor::workspace::get` | The active root, open buffers, and expanded folders — what every surface sees. |
| `editor::tree` | List a folder, with the workspace's expansion state. The walk, the noise-folder excludes and the jail are shell's. |
| `editor::open` | Read a text file and record it as an open buffer, with its language id and the mtime to save against. |
| `editor::save` | Whole-file write, refused when the file moved since the open it started from. The refusal carries the disk-vs-yours diff. |
| `editor::buffers::list` | Files currently open. |
| `editor::buffers::close` | Close one buffer. The file on disk is untouched. |
| `editor::move` | Move or rename, then rewrite every open buffer and expanded folder at or under the path. |
| `editor::create` | Create a file or folder, parents included. A file may be seeded with content. |
| `editor::delete` | Remove a path and close any buffer it held. |
| `editor::find` | Fuzzy file finder, ranked basename-first. Candidates from git in a repo, from the folder listing otherwise. |
| `editor::search` | Search file contents across the workspace, grouped by file. shell's recursive grep, shaped for a results panel. |
| `editor::diff` | Unified patch between two texts. Pure — no path is read, so it works on content that is not on disk yet. |
| `editor::git::status` | Branch, upstream, ahead/behind, and one typed row per changed path. |
| `editor::git::hunks` | What changed in one file: the rendered patch plus its line ranges. |
| `editor::git::show` | A file's contents at a revision, HEAD by default. Pair it with the working copy to render a diff without parsing a patch. |
| `editor::git::commit` | Stage and commit. `committed: false` when there was nothing staged. |
| `editor::git::sync` | Fetch, fast-forward pull, or push, with ahead/behind after. |
| `editor::git::stash` | Stash the working tree, or pop the most recent stash. |
| `editor::git::undo-commit` | `reset --soft HEAD~1`, returning the SHA and message undone. |

Pull is `--ff-only` on purpose: a merge under open buffers is how an editor
ends up showing a conflicted tree nobody asked for. A repository that needs
interactive credentials will hit `git_timeout_ms` rather than hang, because
`shell::exec` owns the process.

Two are worth calling out. `editor::diff` is the one an agent reaches for most:
it can show exactly what a write will change before making it. And `editor::move`
exists because `shell::fs::mv` alone leaves open buffers pointing at the old
path — the next save then writes them back there, silently recreating the folder
that was just moved.

## Console page

`#/ext/editor` is a view over the same workspace: a collapsible file tree on
the left with a files/search switch, tabs and the shared Monaco editor on the
right, an edit/diff toggle, and a save that surfaces the conflict guard as a
dialog. A status line under the editor carries the file's language, line count
and git deltas — the shared `CodeEditor` is deliberately chrome-less (no line
numbers, no glyph margin) and the SOP forbids bundling another editor, so that
line is where a gutter's information goes. A git strip along the bottom does
commit, fetch, pull, push, stash and pop. Folder expansion round-trips through the worker, so it
survives a reload and both surfaces agree on it. It polls the working tree, so a
file an agent edits lights up as the edit lands and an open tab you have not
typed in reloads under you. A tab you *have* edited is never reloaded; it is
flagged, and the conflict guard decides the outcome.

`editor::*` calls also render as themselves in chat and traces: a diff as a
diff, a save as a file card with its line counts.

The diff view is rendered by the worker rather than borrowed from the
console's own diff cards. Those are backed by a library that is already
loaded in the console but is not exposed through `@iii-dev/console-ui`, and
bundling a second copy into this worker's asset measures at 10.3 MB, over
the 8 MiB per-asset cap. So the page draws its own: one row per line, added
and removed lines banded, hunk headers, and the file's real line numbers
taken from the `@@` headers. It gives up syntax highlighting inside the
diff. If a shared diff component is ever added to `@iii-dev/console-ui`,
this page should use it and drop the local renderer.

## Configuration

Runtime config lives in the `configuration` worker under id `editor`, so the
console's Workers tab can edit it and every field hot-reloads — handlers read
the live snapshot per call, and nothing needs a restart. There is no committed
`config.yaml`; these are the defaults seeded when nothing is stored yet, and
`--config <path>` is an optional one-time seed that never overwrites a stored
value.

Every field is a bound. Nothing here grants access — that is `shell`'s config.

```yaml
max_diff_bytes: 2000000     # per side of editor::diff, bytes
diff_context_lines: 3       # unchanged lines kept around each hunk
find_limit: 50              # rows returned by editor::find
max_find_candidates: 50000  # paths scanned per editor::find call
max_file_bytes: 2000000     # largest file editor::open will pull back
search_max_matches: 2000    # matching lines editor::search collects
git_timeout_ms: 15000       # per git invocation handed to shell::exec
```

## Local development & testing

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
cargo test
```

`--url` also reads `III_URL`, which the worker manager injects — inside a
sandbox the engine is on the VM's gateway, never on its loopback.

The console assets are built from `ui/` by `build.rs`. For the hot-reload loop:

```bash
cd ui && pnpm install && pnpm watch    # esbuild --watch → dist/
III_EDITOR_UI_WATCH=1 cargo run        # re-registers each changed asset
```
