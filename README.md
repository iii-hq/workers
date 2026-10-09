# Kits — ADE install and update evidence

Recorded on 2026-10-09 from branch `feat/kits` (`iii-directory`).
- **Setup:** the global iii 0.24.4 runs an engine with the production `ade` and `kanban` 0.1.18. The local `iii-directory` build is a path worker pointed at a local registry (iii-hq/registry `feat/kits`) that serves `iii-hq/kanban-team` 1.0.0 → 1.1.0.
- **Starting state:** the kit is not installed, and kanban's own agent profiles are on disk.
- **Format:** captures are 1440×900 in the light theme, with a drawn cursor.

## Videos

| File | What it shows |
|---|---|
| [`ade-install.mp4`](ade-install.mp4) | 1m40s. Empty Kits tab → `iii trigger directory::download-kit kit=iii-hq/kanban-team@1.0.0` in a terminal → the badge and banner appear live → install review (overwrite warnings, diff, permissions, previews, per-collision choice) → progress → done → installed kit → Agent Profiles with their origin. |
| [`ade-update.mp4`](ade-update.mp4) | 1m48s. Two skills edited on disk → "View my changes" → update check → PR-style update review (changed profile, new profile, workers, skills, permissions) → conflict resolved across the three tabs → Update enabled → progress with `compose::add web@^1.2` → done. |
| [`ade-install-dark.mp4`](ade-install-dark.mp4) | The install flow in the dark theme. |
| [`ade-install-highlights.mp4`](ade-install-highlights.mp4) | An edited 66s cut of the dark install, with music. |

[`ade/terminal-download-kit.txt`](ade/terminal-download-kit.txt) is the full terminal output of the `download-kit` command: the plan JSON and where to review it.

## Screenshots (`ade/`)

| File | What it shows |
|---|---|
| `01-kits-tab-before-install.png` | Kits tab before install: nothing installed, "Install a kit" |
| `02-pending-install-banner.png` | Right after the terminal command: badge, "Needs review", pending banner |
| `03-install-review-warnings.png` (`--full`, `--dark`) | Install review: "2 existing profiles will be replaced — came from worker kanban 0.1.18", permissions, profiles / skills / workers |
| `04-install-review-view-diff.png` | "View diff": the worker's profile vs the kit's |
| `05-install-review-permissions.png` | Permissions expanded: the functions the profiles preload |
| `06-install-review-profile-preview.png` / `07-install-review-skill-preview.png` | Previews: frontmatter as fields plus the markdown body |
| `08-install-review-per-collision-choice.png` | One collision set to "Keep the current one"; the button now says "Install and replace 1 profile" |
| `09-install-in-progress.png` / `10-install-done.png` | Steps Workers → Files → kits.lock; done |
| `11-installed-kit-overview.png` (`--full`) … `15-installed-kit-files.png` | Installed kit: overview, profiles, skill preview, workers, files |
| `16-agent-profiles-origin.png` | Agent Profiles list showing each profile's origin (kit, worker, kept local) |
| `17-locally-edited-files.png` / `18-view-my-changes.png` | Two skills edited on disk show as "edited"; "View my changes" diffs them against the installed base |
| `19-check-updates-badge.png` | Update check: 1.1.0 available, badge on the Kits tab |
| `20-update-review-overview.png` (`--full`, `--dark`) | Update review laid out like a PR: grouped changes, release notes, "minor" badge, Update blocked by 1 conflict |
| `21-update-changed-profile.png` | Changed profile: frontmatter summary (model sonnet → opus, + `web::fetch`, + skill), then the split diff |
| `22-update-new-profile.png` | New profile preview |
| `23-update-new-worker.png` / `24-update-changed-range.png` | Worker `web` added; `kanban` range changed |
| `25-update-skill-clean-merge.png` | Skill edited locally and changed by the kit, merged cleanly |
| `26-update-removed-skill.png` | Removed skill: remove it or keep it as a local file |
| `27-update-permissions.png` | What the update newly grants |
| `28-conflict-kit-changes.png` … `31-conflict-result-edited.png` | Conflict: kit's changes, your changes, result with markers, result edited by hand |
| `32-update-enabled.png` … `34-update-done.png` | Update enabled after resolving; progress; done |
| `35-files-after-update.png` / `36-workers-after-update.png` | Files and workers after the update |
