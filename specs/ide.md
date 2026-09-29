# ide

## Problem
In the IDE's Source control view, an operator with many changed files under one folder must stage (or unstage, or discard) them one file at a time. In the tree view, folder rows only expand and collapse; they have no actions.

## Outcome
In the tree view, every folder row in "Changes" offers Stage and Discard for everything under it, and every folder row in "Staged changes" offers Unstage for everything under it, exactly like a file row does for one file (VS Code behaviour).

## Users and the primary object
The operator using the IDE page in the ADE. The object is the set of Git changes (worktree and index) of the browsed repository, read and changed through `git` over `shell::exec`.

## Console surface
Existing IDE page, Source control view, tree mode (`View as Tree`). Folder rows gain the same hover/focus action buttons file rows have, at the right of the row:
- in Changes: Discard changes (opens the existing confirmation listing the files, count = files under the folder) and Stage changes;
- in Staged changes: Unstage changes.
Buttons are reachable by keyboard (Tab) and on touch/narrow panes the same way file-row actions are. Clicking the folder name still expands/collapses. While an action runs, all actions are disabled (existing busy state); a git failure shows in the existing note line. List mode is unchanged (it has no folder rows).

## Functions
No new worker functions. The page keeps using its existing git actions (stage, unstage, discard over a list of paths).

## Live updates
After the action, the lists refresh as they do today after a file action: the folder's files move between Changes and Staged changes.

## Configuration
None.

## Acceptance criteria
1. Operator, in tree view, clicks Stage on a folder in Changes → all files under that folder (including subfolders) appear under Staged changes and no longer under Changes; files outside the folder are untouched. Verify: in a scratch repo with changes in `a/x.txt`, `a/b/y.txt` and `z.txt`, open `#/worker/ide`, Source control, View as Tree, Stage on `a` → Staged shows `x.txt` and `y.txt`, Changes shows only `z.txt`; `git status --porcelain` agrees.
2. Operator clicks Unstage on a folder in Staged changes → all files under it return to Changes. Verify: continue from 1, Unstage on `a` → Staged section disappears; `git status --porcelain` shows nothing staged.
3. Operator clicks Discard on a folder in Changes → a confirmation names the files under it; Cancel changes nothing, Discard reverts only those files. Verify: in the scratch repo, Discard on `a`, Cancel → unchanged; Discard again, confirm → only `z.txt` remains changed.
4. The folder actions are keyboard reachable and file-row actions, file clicks and folder expand/collapse keep working; list view looks as before. Verify: Tab onto a folder's Stage button and press Enter → same result as 1; click a folder name → it collapses; toggle View as List → no folder rows, file actions unchanged.
5. Operator clicks the row of a changed image (png, jpg, gif, webp…; SVG is editable markup and keeps its diff) in Source control, in Changes or Staged changes, list or tree view → the image opens directly in the editor, without the "This is a binary file" diff step; a text file row still opens its diff, and a deleted image still opens the diff notice (there is no file to show). Verify: in the scratch repo add `img/pic.png` and change `z.txt`, open `#/worker/ide`, Source control, click `pic.png` → the image tab shows the picture; click `z.txt` → its diff opens.

## Out of scope
- Opening images directly from the Timeline tab (still opens the turn's diff).
- Folder actions in list view (it has no folder rows).
- Staging from the Files explorer tree or by context menu.
- Partial (hunk/line) staging.

## Project context
- Source control UI: `ide/ui/src/page/SourceControlTab.tsx` (file rows, actions), `ide/ui/src/page/ChangeEntries.tsx` (tree layout, `DirectoryRow` has no actions), `ide/ui/src/page/scm-view.ts` (`buildChangeTree`).
- `use-source-control.ts`: `stage(paths[])`, `unstage(paths[])`, `discard(entries[])` already take lists; `git-actions.ts` stages with `git add -A -- <paths>`.
- No prior spec for `ide`; `ide/ARCHITECTURE.md` documents the modules.

## Architecture
UI-only. `ChangeEntries` gains optional `renderDirectoryActions(entries, directory)`; entries = every file under the folder (`directoryEntries`, depth first). With it, a folder row renders as `.shui-scm-row.shui-scm-folder-row` (toggle button + `.shui-scm-row-actions` + empty status cell), reusing the file-row hover/focus visibility. Without it (Timeline) markup is unchanged. `SourceControlTab` passes Unstage (staged) and Discard + Stage (unstaged) over the existing `scm.unstage/stage(paths)` and a new `PendingDiscard` kind `folder` for the confirm title. UI assets are embedded in the Rust binary at compile time (`ide/src/ui.rs` include_str!), so delivery needs a rebuild + restart of the ide worker.

## Progress
- Class: UI-only
- Plan: done (user confirmed) · harness/ade-solo/plan · Architect: skipped (UI-only; no contract change) · Backend: skipped (no service change) · Frontend: in progress (code done; layers 2-4 blocked) · Accept: pending
- Next: user decides how the running IDE picks up this checkout (the compose project `my-project` runs `cargo run --bin ide` from /home/layon/workspaces/worktrees/workers-main-20260919/ide, not from this repo); then rebuild/restart the ide container and run layers 2-4 + Accept
- Evidence: `specs/ide.evidence.md`

## Notes
- Assumed: Discard on a folder is included (with confirmation), matching VS Code, since file rows have it.
