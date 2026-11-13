# tmux

tmux as an iii worker. It lists every tmux session, window and pane on the
host, creates, renames and kills sessions and windows, moves windows between
sessions, and tells you what is running where: which pane holds Claude Code
or Codex and whether that agent is working, waiting for you or idle, which
pane runs `iii compose` and on which port its engine listens, and every TCP
port your processes listen on, traced back to the pane that started them. It
reads how much of their weekly allowance Claude Code and Codex have used, and
runs Claude Code on several accounts, moving a pane from one to another
between turns. Its Console page embeds a terminal that is a real tmux client:
what you type goes straight into the session, and the screen updates as tmux
draws it, the same way a second terminal attached with `tmux attach` would.

## Install

```bash
iii call compose::add worker=tmux
```

The worker drives the `tmux` binary installed on the host (Linux or macOS) and
talks to the same tmux server you use from your terminal. Without tmux it still
starts and reports `available: false`, and `tmux::install` can put it there.

## Installing tmux

```bash
iii call tmux::install               # starts or joins the install, answers at once
iii call tmux::install::status       # poll until succeeded or failed
```

| Computer | What runs, in order |
|---|---|
| macOS | Homebrew when `brew` is there (never as root), otherwise MacPorts as root or with `sudo -n`; then the official build |
| Linux, as root or with `sudo -n` | The first of `apt-get` (after `apt-get update`), `dnf`, `pacman` (never `-Sy`, so a stale or empty database falls through), `zypper`, `apk`; then the official build |
| Linux, no root and no passwordless sudo | The official build |
| Linux on a processor with no official build (armv7) | Only a package manager, and without root or sudo the error says what to ask an administrator to run |

Nothing ever waits for a password or for an answer. The official build is
[`tmux/tmux-builds`](https://github.com/tmux/tmux-builds) 3.7c for Linux and
macOS on x86_64 and arm64, pinned by SHA-256 in this worker's source. It goes
to `~/.local/bin/tmux`: checked before it is unpacked, asked `-V` before it
lands there, moved atomically. A file already at that path is never replaced.

An attempt only counts when tmux then resolves, answers `-V` and starts a
server on a private socket of its own, and one that reports a failure counts
when tmux works anyway. A package manager that fails falls through to the
official build, and the failure's `log` says so. With tmux already there the
install changes nothing: it makes the same check and answers `succeeded` with
`method: null`. An install never talks to your tmux server.

The answer of both functions:

```json
{ "state": "running", "step": "downloading", "label": "Downloading tmux…",
  "hints": ["From the tmux project's official builds", "tmux 3.7c"],
  "progress": 0.42, "method": "prebuilt", "path": null, "version": null,
  "error": null, "log": [] }
```

`state` is `idle`, `running`, `succeeded` or `failed`; `step` is `checking`,
`updating`, `downloading`, `verifying`, `installing` or `finishing`; `method` is
`apt`, `dnf`, `pacman`, `zypper`, `apk`, `homebrew`, `macports` or `prebuilt`.
`label` and `hints` are the words to show while it runs, `error` is one
sentence, and `log` is empty until a failure, when it holds every command and
its output. A worker that restarted mid-install answers `idle`.

`tmux::status` says `installable: true` when tmux is missing and this can try:
Linux or macOS, with `tmux_binary` left as plain `tmux`, on a processor with an
official build or a computer with one of those package managers.

A daemon's `PATH` is often short, so plain `tmux` is also looked for after
`PATH` in `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`,
`/home/linuxbrew/.linuxbrew/bin` and `/opt/local/bin`.

On a system with no terminfo entry for `xterm-256color` (Alpine without
`ncurses-terminfo-base`), the official build can run every command but can't
attach a terminal, so the install also writes that one entry to
`~/.terminfo/x/xterm-256color`.

`tmux --install-tmux` runs the same install with no engine and exits: the steps
and the log on stderr, the final status as the last line of stdout.

## Quickstart

Open the **tmux** page in the Console to browse sessions, open one in the
terminal, and see the **Ports** tab. From code or an agent:

```bash
# Every session → window → pane, with agent / compose / port status
iii call tmux::sessions::list

# A new session running a dev server, then a second window in it
iii call tmux::sessions::create name=api cwd=~/code/api command="pnpm dev"
iii call tmux::windows::create session=api name=logs

# Who listens on which port, and which compose stacks are up
iii call tmux::ports::list
```

Names are taken as typed: tmux 2.7 and later read `#` in a session or window
name as a format (`#{…}`, or `#(…)`, which runs a command), so the worker
doubles it there, and only there. A name shaped like an id (`$2`, `@3`, `%4`)
is refused, since a session is named by its id or its name. A session named
like that outside the worker never stands in for the session with that id:
when `$2` is one session's id and another's name the call is refused, saying
which session to rename. Refusals are one sentence, `There is no pane %9.`
rather than tmux's `can't find pane: %9`.

A new window without `cwd` starts in the directory its session was created
in. tmux older than 3.2 can't tell a session's directory, so there it starts in
the home folder.

A pane whose program ends closes, as in tmux, and so does its window and its
session when it was their last. With `remain_on_exit: true`,
`tmux::sessions::create`, `tmux::windows::create` and `tmux::panes::split`
keep the new pane on screen once its program ends, `dead` in the listing, for
`tmux::panes::respawn` to start again. The pane first runs a placeholder,
then remain-on-exit is turned on and the command starts in its place, so even
a command that fails at once stays. It is the pane's own option from tmux
3.0; on tmux 2.x, which has no pane options, every pane of its window stays.

```bash
iii call tmux::windows::create session=api name=build command="npm run build" remain_on_exit=true
```

A pane running an agent and a pane running `iii compose` look like this in
`tmux::sessions::list`:

```json
{ "id": "%3", "command": "claude",
  "agent": { "id": "claude", "label": "Claude Code", "state": "waiting", "since_ms": 1791936000000,
             "title": "Fix the login redirect" } }
{ "id": "%7", "command": "iii:c:default",
  "compose": { "namespace": "default", "pid": 2759592 },
  "ports": [ { "port": 49135, "role": "engine", "process": "iii:e:default" },
             { "port": 3111, "role": "worker", "worker": "http" },
             { "port": 3114, "role": "worker", "worker": "ade" } ] }
```

Agent state comes from the pane's last lines: a permission or choice prompt is
`waiting`, Claude Code's `esc to interrupt` line is `working`. Claude Code and
Codex also count as `working` while their sub-agents run after the main turn
ended, which their screen does not show; the worker reads what they record on
disk (Claude Code's `~/.claude/sessions/<pid>.json` and each sub-agent's meta
file and transcript, the `rollout-*.jsonl` files Codex holds open) and lists
those sub-agents in `subagents`:

```json
{ "id": "%9", "command": "codex",
  "agent": { "id": "codex", "label": "Codex", "state": "working", "since_ms": 1792698537000,
             "title": null,
             "subagents": [ { "id": "01a16528-d646", "description": "review/tests", "kind": null,
                              "depth": 1, "since_ms": 1792698537000 } ] } }
```

The Console page shows the count on the agent's badge and a row per sub-agent
under its window. A sub-agent has no pane of its own: selecting its row opens
its transcript, which `tmux::subagents::transcript` returns from the same
files. That is the task it was given, what it wrote and reasoned, and the
tools it called with what they returned, oldest first, while it runs and
after it finished:

```bash
iii call tmux::subagents::transcript pane=%9 subagent=01a16528-d646
```

```json
{ "pane_id": "%9", "agent": "codex", "subagent_id": "01a16528-d646", "working": true,
  "prompt": "Message Type: NEW_TASK\nTask name: /root/review/tests\n…",
  "items": [ { "kind": "tool_call", "at_ms": 1792698540000, "text": "cargo test",
               "tool": "shell", "error": false },
             { "kind": "tool_result", "at_ms": 1792698551000, "text": "test result: ok…",
               "tool": "shell", "error": false } ],
  "truncated": false }
```

Item kinds are `user` (a later message from the main agent), `text`,
`thinking` (when the agent records it in clear), `tool_call` and
`tool_result`. Each text is cut at 4 000 characters, and only the last 4 MiB
of a long transcript are read, plus its task. Codex keeps a sub-agent's task
encrypted, so there the prompt shows only its name and sender.

The agent's
`title` is the session title it put on its pane (Claude Code's conversation
topic) without the ✳ or spinner in front; it is null while the pane shows
tmux's default title, the host name. Selecting a window
(`tmux::windows::select`) works like tmux itself: every client attached to
that session, your own terminal included, switches to it.

`tmux::windows::move` moves a window to another session, or next to one of
its windows. The window keeps its id, panes and programs:

```bash
# After the last window of `work`
iii call tmux::windows::move session=api window=@3 target_session=work
# Just before (or after) window 0 of `work`; in the window's own session this reorders it
iii call tmux::windows::move session=work window=@3 target_session=work before=0
```

```json
{ "session_id": "$2", "window_id": "@3", "window_index": 0, "source_session_closed": false }
```

Without `before` or `after` the window goes after the session's last window,
even when an index between two windows is free (tmux's own `move-window -t
work:` would take that gap): from tmux 3.2 that is `move-window -a` after the
last window, decided by tmux in one step; before 3.2 the worker names the
index after the last one, and should another client take it first the answer
says to move it again. `before` and `after` are tmux's `move-window -b` and
`-a`: the window takes its neighbour's index (or the next one), and the
windows from there up to the first free index move up one, so a window past a
gap keeps its index. Both need tmux 3.2 or later, and an older tmux gets one
sentence saying so: `-b` came in 3.2, and before it `-a` put the window after
the session's current window whatever the target, and inside one session
ended the tmux 2.6 and 3.0a server. The
destination's current window stays as it was, and a session's current window
reordered inside it stays current. When the window was the last one of its
session, tmux closes that session and `source_session_closed` is true; with
tmux's default `detach-on-destroy on`, clients attached there (a Console
terminal too) end, so switch a terminal to the target session first. The
window already in the target session without `before`/`after`, both at once,
or a window next to itself are refused in one sentence.

Panes work the same way. `tmux::panes::select` makes a pane active, for every
client looking at that window. `zoom` sets the window's zoom and never toggles
it the way prefix `z` does: `zoom: true` zooms the window on that pane (or keeps
it zoomed there), `zoom: false` unzooms it, and without `zoom` the window keeps
its zoom, moved to that pane. A window with one pane can't zoom, and the
answer's `zoomed` stays false. tmux itself only toggles a zoom, so this is a
read and then a toggle, not one step: another client can zoom or unzoom the
window in between. The worker reads the zoom again after toggling, toggles
once more if it doesn't match, and the answer says what it ended as.
`tmux::panes::capture` returns a pane's visible
screen with its colour escapes, its size and cursor:

```bash
iii call tmux::panes::select pane=%3 zoom=true
iii call tmux::panes::capture pane=%3
```

`tmux::panes::split` puts a new pane to the `right` of a pane or `below` it. It
starts in that pane's current directory unless `cwd` says otherwise, runs the
default shell unless `command` does, and becomes the active pane unless
`select: false`; `remain_on_exit: true` keeps it once its program ends (see
above). `tmux::panes::kill` kills a pane and its program; when that was
its window's last pane the window closes too (`window_closed`), and when that
window was its session's last, the session (`session_closed`).
`tmux::panes::respawn` starts a pane's program again in the same pane, the
command it was started with unless `command`. A pane whose program still runs
is refused unless `kill: true`; one kept on screen after its program ended
(tmux's `remain-on-exit`) is dead and restarts without it.

```bash
iii call tmux::panes::split pane=%3 direction=below command="npm test" select=false
iii call tmux::panes::respawn pane=%7 kill=true
iii call tmux::panes::kill pane=%7
```

```json
{ "session_id": "$1", "window_id": "@4", "pane_id": "%7" }
{ "pane_id": "%7" }
{ "killed": true, "window_closed": false, "session_closed": false }
```

## Claude Code accounts and usage

The worker reads how much of their weekly allowance Claude Code and Codex
have used, and keeps several Claude Code accounts side by side. One of them
is the **global** account: the one signed in to `~/.claude`, which every
Claude Code that isn't pinned uses, plain `claude` typed in any shell
included. Switching it works like `/login`: the sign-ins change places, and
every session on `~/.claude` uses the new account with its next message,
keeping its conversation, with no restart. A pane can also be pinned to one
account; pinned panes move between accounts between turns, resuming their
conversation.

```bash
# Accounts, their usage and reset times, Codex's usage, the rules and any suggestion
iii call tmux::usage::read                    # refresh=true reads again now
iii call tmux::accounts::list

# A second account, then sign it in: open the link it answers, paste the code the page shows
iii call tmux::accounts::add name=Personal color=green
iii call tmux::accounts::login_start account=personal
iii call tmux::accounts::login_code login_id=<login_id> 'code=<code#state>'

# A window running Claude Code on that account, or following the global one
iii call tmux::windows::create --json '{"session":"work","claude":{"account":"personal"}}'
iii call tmux::windows::create --json '{"session":"work","claude":{"account":"global","skip_permissions":true}}'

# Make a pane follow the global account, make another account global
iii call tmux::panes::account pane=%5 account=global
iii call tmux::accounts::global account=personal
```

### Accounts

Every account has a folder under `~/.local/share/iii/claude-accounts`
(`$XDG_DATA_HOME/iii/claude-accounts` when that is set), `default` included
(`…/claude-accounts/default`), listed with its name, colour and what it
shares in `accounts.json` there (0600, written whole, then renamed into
place; its `revision` goes up by one on each change the worker makes).
`default` is the account `~/.claude` was signed in to when accounts began.
Without `accounts.json` there is one account, `default`, on `~/.claude`, and
the worker changes nothing.

An account's sign-in (its chain of tokens) lives in exactly one place: in
`~/.claude` while it is the global account, in its own folder otherwise.
Two copies of one chain would part on the first token refresh, and Claude
Code would sign the losing copy out, so the worker always moves a sign-in,
never copies it. Each folder's `.claude.json` keeps the account's profile
(`oauthAccount`), which is how the worker knows who is signed in where.

`tmux::accounts::add` takes a `name` (no two alike, case aside), a `color`
token (`blue`, `green`, `violet`, `orange` or `pink`; the apps draw anything
else gray) and `shared`. The id is the name as a slug (`[a-z0-9-]`, at most 32
characters, never `default` or `global`); a folder of that name left in the
accounts folder is never reused, the id gets `-2`. The new folder is 0700 and
signed out.

`shared` names what the account's folder takes from `~/.claude` through
symbolic links (all four groups unless it says otherwise; `default` shares
all four):

| Group | Links |
|---|---|
| `settings` | `settings.json`, `CLAUDE.md`, `keybindings.json` |
| `extensions` | `skills/`, `agents/`, `commands/`, `output-styles/` |
| `plugins` | `plugins/`, and the user's MCP servers (`mcpServers` of `~/.claude.json`) copied once into the new account's `.claude.json` |
| `conversations` | `projects/`, `history.jsonl`, `todos/`, `plans/` |

A link is made only for what exists in `~/.claude`, and never in place of a
file or folder of the account's own; links missing are made again when the
worker starts and on each update or sign-in. `.credentials.json`,
`.claude.json`, `sessions/`, `statsig/` and `shell-snapshots/` are never
linked. `tmux::accounts::update` renames, recolours or changes the groups:
dropping one removes only the worker's own links into `~/.claude`.

An account signs in through `tmux::accounts::login_start` and
`tmux::accounts::login_code` ([Signing in](#signing-in)). The worker never
signs in, refreshes or signs out by itself.

`tmux::accounts::remove` takes an account off the list; not `default`, not
the global one, not one a switch waits to make global. Claude Code panes on
its folder keep running. `delete_files: true` deletes its folder too, only
when it is a real folder directly inside the accounts folder (a link is never
followed out of it), and is refused while Claude Code runs on it (a pane,
or a sign-in from the app, until its Claude Code is gone), while any pane is
being moved, or when tmux could not list its panes; it looks at the
processes again first, never at a scan from a few seconds before.

### Signing in

`tmux::accounts::login_start` (`account`, `browser`) runs `claude auth login`
for an account in a terminal of the worker's own, which no tmux session
shows: on the account's folder (`CLAUDE_CONFIG_DIR=<folder>`), or on
`~/.claude` for the global account (without `CLAUDE_CONFIG_DIR`), with
`--email` when the account's profile has one. It answers once Claude Code
printed its sign-in link, with `login_id`, `account`, `url`, `expires_at_ms`,
`browser`, and `email` and `browser_url` when there are. `url` is the link
that works on any device: the page ends showing a code, which the person
copies back. A new `login_start` for the same account ends the one under
way; of two started at once, the later one stays and the other answers
`gone`. Claude Code runs in the worker's home folder, with the worker's
`HOME`. While a switch of the global account is left half-way and not
settled yet, no sign-in starts on `~/.claude` or on the two accounts it
moves (`failed`, try again in a moment).

`browser` says where a browser may open:

- `none` (the default): nowhere. Claude Code runs with the worker's own
  `BROWSER`, a script in a temporary folder (0700) that notes the link and
  opens nothing, and with that folder's `open` and `xdg-open` first on its
  `PATH`, so it never opens a browser on the worker's computer. The person
  opens `url` wherever they are and pastes the code.
- `host`: the browser of the computer the worker runs on, for someone
  sitting at it (the Mac app's "This Mac"). The link Claude Code opens goes
  to the worker's own `$BROWSER`, else `open` (macOS) or `xdg-open` (with
  `DISPLAY` or `WAYLAND_DISPLAY` set, as Claude Code itself asks), and
  comes back as `browser_url`: it returns to Claude Code's listener on
  `127.0.0.1` and signs in by itself, with no code. `url` and a code work
  too. Only `login_start`'s answer carries `browser_url` (the list and the
  feed, which need no approval, never do), and `host` fails when nothing on
  that computer opens a browser.

`tmux::accounts::login_code` (`login_id`, `code`) types the code into that
terminal. Whitespace anywhere in it is dropped (a code copied over two
lines); one without its two halves around `#` is refused as `invalid_code`
before it reaches Claude Code, and the sign-in keeps waiting. It waits up to
60 seconds for Claude Code and answers `{"state": "done", "account"}`, the
account read again, or an error; of two calls at once, one types its code
and the other waits for that outcome. `done` means a new sign-in is where it
belongs: in the account's folder, or on `~/.claude` and taken in as the
global account (looked for three times, a second apart); Claude Code saying
it signed in while the sign-in there is still the one from before is
`failed`.
Whoever signed in stands, even someone other than the email the page was
filled with: `account` shows who. A code Anthropic refuses ends the sign-in
(Claude Code exits): another `login_start` gives a new link, and only a code
from that link works with it. `tmux::accounts::login_cancel` (`login_id`)
ends one and answers `{}`, also when it is over or unknown. A sign-in not
finished within 10 minutes ends as `expired`, and a code typed after that
is `expired` too. While a code is being checked, Claude Code may be saving
the sign-in, so neither the 10 minutes, a cancel nor `login_code` giving up
stops it before 15 more seconds; its own outcome stands when it comes by
then. With `host`, the 10 minutes running out gets the same 15 seconds: the
person may have just come back from the page. Claude Code is stopped with
SIGTERM, and with SIGKILL 3 seconds later if it is still there. Removing an
account ends its sign-in.

Errors come as one JSON object in the error's message, `{"code",
"message"}`:

| `code` | When |
|---|---|
| `invalid_code` | not shaped like a code; nothing typed, the sign-in keeps waiting |
| `rejected` | Anthropic refused the code (Claude Code's own sentence); the sign-in is over |
| `expired` | 10 minutes went by |
| `gone` | cancelled, replaced by a newer `login_start` for the account (that start's own answer too), the account removed, the worker stopping, or unknown; the message says which |
| `failed` | Claude Code is not installed, could not start, printed no link within 45 seconds, ended without signing in (the person cancelled on the page: its sentence) or as if signed in with no new sign-in to show; `host` with no browser to open; a switch left half-way on the places it would sign in |

`tmux::accounts::list` and the feed's `usage.claude` carry `logins`: the
sign-ins under way and those that ended in the last minute, each with
`login_id`, `account`, `state` (`waiting_code`, `checking`, `done`, `failed`
or `expired`), `url`, `expires_at_ms`, `browser`, and `email` and `error`
when there are. A cancelled one leaves the list at once. A sign-in on
`~/.claude` to someone else is a sign-in by hand there: it makes that
account global ([below](#the-global-account)). While a sign-in runs on
`~/.claude` or on an account's folder, `tmux::accounts::global` refuses to
switch the global account away from or to the accounts it involves
(`signing_in`), and the rules and a waiting switch go on once it is over:
a swap would move sign-ins where Claude Code is about to save one. A
cancelled sign-in counts until its Claude Code is gone (one checking a code
gets its 15 seconds to save it).

The code and the tokens are never logged, kept or answered; the worker's
log shows a link without its `state`, `code_challenge` and `login_hint`, and
Claude Code's output stays in memory only while the sign-in runs. On macOS
Claude Code keeps the sign-in in the login Keychain and, when that Keychain
is locked, does not fall back to a file: the worker has to run in the
person's logged-in session (a LaunchAgent, or a terminal there), not as a
LaunchDaemon or from an ssh session, or the sign-in fails.

### The global account

The global account is whoever is signed in to `~/.claude`: its
`.credentials.json` (on macOS the login Keychain item `Claude Code-credentials`)
and the `oauthAccount` profile in `~/.claude.json`. `tmux::accounts::global`
makes another signed-in account global by swapping sign-ins, under Claude
Code's own locks and in its order, so a Claude Code renewing its token never
sees half a swap:

1. The locks: `~/.claude/.oauth_refresh.lock`, `~/.claude.lock`,
   `~/.claude/.storage-write.lock`, then the same in the two account folders
   (directories made with `mkdir`, as Claude Code's lock library makes them;
   one older than its stale period is taken over). If Claude Code holds one
   for more than 10 seconds, nothing changes and the call fails with `busy`.
2. The old global account's newest sign-in (it may have been renewed while it
   was global) goes from `~/.claude` to its folder; the new one's goes from its
   folder into `~/.claude`, and is removed from its folder. A sign-in is the
   store's keys that belong to the Anthropic account, the ones Claude Code's
   logout removes, each moved if present: `claudeAiOauth`, `designOauth` and
   `cloudSessionsOauth` (three token chains Claude Code revokes together),
   `organizationUuid`, `trustedDeviceToken`,
   `trustedDeviceEnrolledAfterRefusalAtMs` and `enterpriseGateway`. Every
   other key stays where it is: `mcpOAuth` holds
   sign-ins to MCP servers, which are yours, not the Claude account's, and
   stays in `~/.claude`. Files are written to a temporary file (0600), synced
   and renamed. On macOS the Keychain item is written the way Claude Code
   writes it (`security -i`, the secret on standard input). A store over
   4032 bytes goes as arguments instead, hex-encoded with its tokens, where
   another process of yours could see it while `security` runs: the same as
   Claude Code does, and only for a store over 4 KB. A plaintext
   `~/.claude/.credentials.json`, when there is one, gets the same change.
   The locks are touched before every store call, and a swap that has held
   them for 8 seconds (a Keychain that hangs) stops safely.
3. Under `~/.claude.json.lock`, the old account's profile is kept in its
   folder's `.claude.json`, the new one's goes into `~/.claude.json`, and the
   keys `/login` clears (the old account's cached models, organisation
   defaults, usage and status readings) are cleared.
4. `accounts.json` says the new account is global, with `last_switch`.

No network is used while a lock is held, and nothing is ever signed out or
revoked. Before it starts, the worker writes `swap.json` in the accounts
folder (0600, no tokens: the first 16 hex digits of the SHA-256 of each
refresh token, so it can tell which sign-in is where). A swap left half-way
(the worker stopped, or a write failed: a full disk, a Keychain that timed
out) is settled from what the stores really hold, never from the step it
noted: with the new sign-in in `~/.claude` it is finished; with the old one
still there it is finished, or undone when the switch already answered with
an error; with any other sign-in there (someone signed in by hand meanwhile)
nothing is touched and the sign-in by hand is taken in as below. This
happens at start, within 2 seconds while the worker runs, and before any
switch. The locks a stopped swap left are the worker's own when they are
still the directories it made (device, inode and birth time, or, where the
file system keeps no birth time, touched within its own swap): those go at
once, and anything else is waited out as stale (up to 60 seconds). A stale
lock is moved aside (`<lock>.stale-<id>`) before it is removed. What was
moved goes back when it is not the lock judged stale (a new lock, even one
given the same inode, has new modification and birth times; the old one
may have been touched again), unless something new took its place; what
was moved aside and left goes once stale, at start and before each lock is
taken. The worker releases a lock only while it is still the one it
touched last. A swap and its settling hold `swap.lock` in the accounts
folder (an `flock`, let go of when the process ends) so that a second
worker on the same accounts leaves a swap that runs alone. One switch,
recovery or move runs at a time, and a new Claude Code pane (`claude` on
`tmux::windows::create`, `tmux::sessions::create`) waits for the one running
too: nothing starts on a folder whose sign-in is moving, and nothing plans
on what a swap half-way shows. While a swap left half-way can't be settled
yet, a new Claude Code pane on `~/.claude` or on either account it moves is
refused ("A switch of the global account left half-way is being settled;
try again in a moment."), and no pane moves. Once a call has its turn, its
work (a new Claude Code pane included) runs to the end on its own, even
when the caller goes away (a timeout, a disconnect): a switch is never left
half-way by a call that stopped waiting, and no other one starts before it
is over. A switch or a move needs tmux to list every pane: when it can't
(a timeout), nothing is switched or moved and the error says so. An
account a swap involves (running, or left half-way) can't be removed. A session that
fetched the old profile right before the swap
and writes it back within a minute gets the new one put back. Claude Code
has a feature gate (`tengu_bubbly_fountain`, off today) that would stop a
running session whose account changes; when `~/.claude.json` shows it on,
the switch is refused with `owner_check_on`.

Every Claude Code on `~/.claude` that isn't pinned switches with its next
request (`following`): on Linux at once (Claude Code looks at
`.credentials.json` on every request), on macOS within 30 seconds (Claude
Code keeps what it read from the Keychain that long; `switch_delay_s` in the
snapshot). Pinned panes:

- Pinned to the **new** account (they run on its folder, whose sign-in has to
  move): idle ones are stopped before the swap and started again on
  `~/.claude` after it, still pinned (`restarted`). While one is working or
  waiting, the switch waits: the answer is `state: "waiting"` with
  `waiting_for`, the snapshot carries `pending_global`, and the worker
  finishes the switch within seconds of the last of them going idle. Another
  switch replaces a waiting one; switching to the account that is global
  now cancels it. A Claude Code on that folder the worker can't stop and
  restart (inside another program, sharing a window before tmux 3.0, its
  pane running another program now) refuses the switch with `blocked` and
  `blocked_by`. One started by hand in a shell that does not take the
  terminal back once Claude Code has stopped does not: its folder is free
  then, so the switch goes on, and the pane says it was stopped and not
  started again (`move_error`). When the conversation can't be made visible
  on `~/.claude` (it holds a longer copy), Claude Code starts there with a
  new conversation and `move_error` says so; it never starts again on its
  folder, whose sign-in is in `~/.claude` now. A waiting switch that fails
  in a way that may pass (a lock Claude Code holds, a write that failed, a
  Claude Code slow to stop) is tried again a few times over about a minute,
  and one that meets a sign-in from the app waits for it to end;
  one that can't happen (the account was signed out or removed, a pane
  can't be restarted, `owner_check_on`), or still fails after those tries,
  is called off: `pending_global` goes, and the panes it waited for (and
  any in the way) say why in `move_error` ("The switch to Personal was
  called off: …").
- Pinned to the **old** account (they ran on `~/.claude`): idle ones restart
  on its folder (`restarted`); working ones finish their turn on `~/.claude`,
  now as the new account, and then restart on its folder (`after_turn`).
  One whose Claude Code takes longer than 10 seconds to stop starts again
  on its folder once it ends. One that can't be moved (inside another
  program, sharing a window before tmux 3.0, in copy mode) stays on
  `~/.claude`, so it runs as the new account: it follows the global account
  from then on, counted in `following`, and its `move_error` says so.
- Pinned to any other account: untouched.

Errors come as one JSON object in the error's message, `{"code", "message",
"blocked_by"?}`: `not_signed_in`, `busy`, `blocked`, `owner_check_on`,
`signing_in` (a sign-in from the app runs on `~/.claude` or on either
account's folder) or `failed`.

Someone signing in by hand on `~/.claude` (`/login`, or `claude auth login`
without `CLAUDE_CONFIG_DIR`) changes the global account the same way: the
worker notices within about 2 seconds, makes a known account global
(`last_switch.reason: "login"`) or adds an unknown one, named after its
profile and sharing all four groups, and makes it global (`"login_new"`).
Signing out there by hand leaves the global account as it is, signed out.
A sign-in by hand replaces the global account's only copy of its sign-in
(it lives in `~/.claude` while global), so the account that was global
reads signed out until it signs in again; switching through
`tmux::accounts::global` keeps both. So the panes on `~/.claude` pinned to
the account that was global, which now run as the new one and can't go to
its folder, follow the global account from then on, counted in
`last_switch.following` with the others. For a minute after a switch, the old
account's profile written back over the slot by a session that fetched it
just before is put right, but only while the slot still holds the new
account's sign-in (the one the switch wrote, or one Claude Code renewed it
to); with any other sign-in it is a `/login` and handled as above.

**From 0.3.0.** 0.3.0 kept the global account in tmux's global
environment instead. The first start of this version takes the account
signed in to `~/.claude` as global (`last_switch.reason: "upgrade"`, usually
`default`), gives `default` its folder, pins the panes that followed the
global account onto the folder they run on, and removes the account folder
0.3.0 put in tmux's global `CLAUDE_CONFIG_DIR` (nothing else of tmux's
environment is touched, then or later). Shells opened while 0.3.0 had set it
still carry it in their environment: plain `claude` there runs on that
folder until the shell is closed (`unset CLAUDE_CONFIG_DIR` fixes one).

### Windows on an account

`tmux::windows::create` and `tmux::sessions::create` take
`claude: { account, skip_permissions }` instead of `command`; `account` is
`global` (the default) or an account id. The worker builds the command, marks
the pane, and answers it as `runs`:

| Account | Command |
|---|---|
| `global`, or pinned to the account that is global now | `claude` |
| pinned to any other account | `env CLAUDE_CONFIG_DIR=<folder> claude` |

When the pane's session or tmux's global environment sets
`CLAUDE_CONFIG_DIR`, `claude` becomes `env -u CLAUDE_CONFIG_DIR claude`, and
`runs` says so. The pane's `remain-on-exit` is set as asked, on or off.

`skip_permissions: true` adds `--dangerously-skip-permissions`: Claude Code
then runs commands and changes files without asking. It is never on unless
asked, and is refused when the worker runs as root, where Claude Code refuses
it (`runs_as_root` in the accounts snapshot).

Marks are tmux user options of the pane: `@iii_claude_account` (`global` or
an account id), `@iii_skip_permissions` (`1`) and `@iii_claude_pending` (the
account a move waits to reach). tmux before 3.0 has no pane options, so there
they are options of the pane's window, and a window with more than one Claude
Code pane can't keep an account per pane: its panes are not movable there.
Another pane in a window running Claude Code carries that pane's marks, not
its own, so it gets no `claude` object there, and nothing done to it changes
that pane's account or its waiting move.
Each pane running Claude Code has a
`claude` object in `tmux::sessions::list`: the `account` its process uses (read
from that process's `CLAUDE_CONFIG_DIR`: `~/.claude` is the global account,
so a `claude` started by hand counts too), `config_dir`, `follow` (`global`
on `~/.claude` and not pinned, `pinned` with an account mark, `manual` when
started by hand on an account's folder),
`pending`, `skip_permissions` (the process's own arguments:
`--dangerously-skip-permissions` or `--permission-mode bypassPermissions`;
a mark left from an earlier process in the pane never counts), `session_id`, `movable` with
`not_movable_reason` when it is false, and `move_error` when its last move
failed (cleared by its next account change).

### Moving a pane

`tmux::panes::account` with `account: global` makes a pane follow the global
account (it runs on `~/.claude`), with an account id pins it there (on
`~/.claude` while that account is global, on its folder otherwise; pinning a
pane on `~/.claude` to the account that is global now restarts nothing).
When Claude Code has to run elsewhere for that, the account must be signed
in, and the pane moves:

1. Right before, the worker looks again: the pane must still run the same
   program, with the same Claude Code process under it (its session
   record's start time must be known and be the process's own), in the same
   conversation, with the status `idle`. Otherwise nothing is stopped and
   the move waits for the turn (`after_turn`) or fails.
2. Claude Code gets SIGTERM and up to 10 seconds to end; it saves and runs
   its `SessionEnd` hooks first. A pane the worker started is kept on screen
   meanwhile with a temporary `remain-on-exit`, put back as it was
   afterwards (unset stays unset). When Claude Code takes longer, the move
   fails, but the pane keeps `remain-on-exit` until Claude Code does end;
   then Claude Code starts again on the account it ran on, resuming its
   conversation, and `remain-on-exit` is put back. Started by hand in a
   shell, it just gives the shell the terminal back.
3. The target folder is made to see the conversation: if its
   `projects/<project>/<session>.jsonl` is not the same file (accounts sharing
   conversations link the same folder), the transcript and its `<session>/`
   folder are copied there. A copy already there is only replaced by a longer
   or equally long and newer one; when the account has a longer copy, Claude
   Code starts again on the account it ran on, resuming the conversation, and
   the move fails.
4. Claude Code starts again there with `--resume <session>` (and
   `--dangerously-skip-permissions` again when it had it), the place named
   in the command: `env CLAUDE_CONFIG_DIR=<folder> claude …`, or `env -u
   CLAUDE_CONFIG_DIR claude …` for `~/.claude`. A pane the worker started is
   respawned in place. Claude Code started by hand in a
   shell is started again by typing into that shell (`C-e`, `C-u`, the
   command, Enter), only once the shell has the terminal back within 3
   seconds and still has it right before typing; otherwise nothing is typed,
   the pane is marked as the move would have marked it, and the move fails
   (`move_error`). A shell pane in copy mode is refused before anything is
   stopped, since the keys would go to copy mode. A shell started with `-c` (also `-lc`, `-ec`,
   `--command`) runs one command and ends: its pane is respawned instead.

While a pane moves, a change of its account (`tmux::panes::account`) answers
"This pane is being moved already.".

A pane moves only when Claude Code's own status is `idle`. Working, waiting
for a permission answer, running a background command or with no status, it
answers `after_turn`: the pending mark is set, and the worker moves it within
2 seconds of its turn ending, also after a worker restart (the mark is
tmux's). A waiting move that can no longer happen (its account was removed
or signed out, the pane can't move) is dropped and the pane says why in
`move_error`. Each waiting move is looked at again right before it runs: one
cancelled or changed meanwhile is left to the next round, and a round whose
look at tmux failed changes nothing. Choosing the account it runs on changes no process and cancels
a waiting move (`cancelled`).

`tmux::accounts::move_idle` moves every idle pane using account `from` (its
folder, or `~/.claude` when it is global) to `to` and pins it there: those
that follow the global account, are pinned to `from` or were started by
hand, never one pinned elsewhere.

### Usage readings

Claude Code's usage comes from `GET https://api.anthropic.com/api/oauth/usage`,
the endpoint Claude Code reads its own usage from, with the account's sign-in
(`Authorization: Bearer`, `anthropic-beta: oauth-2025-04-20`), read from
where it lives now: `~/.claude` for the global account, its folder for the
others. It answers each
window (`five_hour`, `seven_day`, per-model weeks) as `utilization` from 0 to
100 with its `resets_at`. The sign-in is `<folder>/.credentials.json` on
Linux; on macOS it is in the login Keychain, a generic password whose account
is `$USER` and whose service is `Claude Code-credentials` for `~/.claude` and
`Claude Code-credentials-<first 8 hex digits of sha256(folder)>` for an
account's folder (the folder exactly as put in `CLAUDE_CONFIG_DIR`,
NFC-normalised, as Claude Code hashes it), with the file as fallback. The worker only reads it: it never
refreshes a token, never sends one anywhere but that endpoint, and never logs
or returns one. An expired sign-in is not used (`token_expired: true`, the
last reading kept; an account that isn't global ages out like this, since
nobody renews its token, and becoming global renews it on its first
request); a window whose reset time has passed reads 0 %
(`reset_since_read`). The reading Claude Code itself left in `.claude.json`
(`cachedUsageUtilization`) is used when it is newer.

Readings are kept at most 5 minutes old while someone watches (a
`tmux::usage::read` call or an observer of `tmux::on-changed` in the last 10
minutes); `refresh: true` reads again at once, at most once a minute per
account. With nobody watching, the worker reads nothing, not even locally
(sign-in files, the Keychain, profiles, Codex's records), unless
`accounts.json` exists and an account other than the global one is signed in
(one that could take over); it looks at who is signed in once when it starts
with `accounts.json`. Then it reads only the global account, every 10
minutes, while the rules are not `off`, and the others when the global one
runs low. A call that moves a pane or switches the global account
(`tmux::panes::account`, `tmux::accounts::global`, `move_idle`) reads the
sign-in of the account it goes to, and that one alone.

A 429 pauses an account's readings for 15 minutes; a 401 counts the sign-in
as expired until its token changes (or the worker starts again); any other
failure is tried again after a minute. The latest readings, without tokens,
are kept in `~/.cache/iii/tmux/usage.json` (`~/Library/Caches/iii/tmux/usage.json`
on macOS).

Codex's usage is read from what Codex records: the newest `token_count` event
of its sessions under `~/.codex/sessions` (the last 4 MB of each file, newest
first, until one is found; at most 8 MB a look), whose windows are told apart by
their length (300 minutes, 10080 minutes) and whose `resets_at` is in seconds.
It comes with the plan and credits; Codex has one account.

### When the global account runs low

`tmux::accounts::rules` sets what the worker does when the global account
reaches `at` (`week_80`, `week_90` or `limit`; `five_hour: true` also counts a
full 5-hour window). With `mode: suggest` the snapshot carries a
`suggestion` (`{ id, from, to, reason, following }`) naming the account to
switch to: the first of `order` that is signed in and has room, otherwise the
one with most room this week (an account whose sign-in expired only when no
other has room). `tmux::accounts::dismiss` answers "Not now": no suggestion
from that account for that reason until its window resets or it runs lower
(week 80 % → 90 % → 5-hour window full → week limit). With `mode: switch` the
worker switches by itself, the app open or not, the same way a person does
(never to an account with Claude Code working on its folder: the next one
instead), and leaves `last_switch` (`auto: true`) for the app; `back_after_reset` (in mode `switch` only, and while it stays
on) switches back once the old account's window resets, unless someone made
another account global since. A switch someone makes leaves `last_switch`
too, with `auto: false` and no `reason`. A switch of the rules happens only on the `accounts.json` they
decided on (any change written meanwhile, even back to the same global
account, drops it until the next look). Until changed, the rules suggest at
`week_90`; `mode: off` stops them.

```bash
iii call tmux::accounts::rules --json '{"mode":"switch","at":"week_90","order":["personal"],"back_after_reset":true}'
iii call tmux::accounts::dismiss suggestion=default:week_90:personal
```

### For tests and demos

| Variable | Replaces |
|---|---|
| `III_CLAUDE_HOME` | `~/.claude`, the global account's place; its `.claude.json` is then the one next to it, in its parent folder, as `~/.claude.json` is next to `~/.claude`. Claude Code started on it finds it through its own `HOME`: give the tmux server the `HOME` that holds it, or panes on the global account use that `HOME`'s `~/.claude` |
| `III_CLAUDE_ACCOUNTS_DIR` | the accounts folder |
| `III_CLAUDE_USAGE_URL` | `https://api.anthropic.com` for the usage endpoint; loopback addresses only, so a sign-in never goes to another host |
| `III_CLAUDE_LOGIN_TTL_S` | the 10 minutes a sign-in may take (10 to 600 seconds) |
| `CODEX_HOME` | `~/.codex` (Codex's own variable) |

[`demo/`](demo/) runs this worker in a container of its own (its own tmux
server, a fake `claude` and `codex`, three fake accounts and a mock usage
endpoint) against an engine on the host: `demo/run.sh <engine-port>`, then
`demo/seed.sh`, `demo/usage.sh` to change what the mock answers and
`demo/state.sh` to make a fake Claude Code work, wait, go idle or hit its
limit, and `demo/ask.sh` to send one a message: it reads its sign-in again,
as Claude Code does, and answers with the account it used (`/login <id>`
typed into it signs in as that account). Its `claude auth login` behaves
like Claude Code's: the code `demo-ok#<anything>` signs the account in as its
own person (`demo-ok:<id>#…` as account `<id>`), any other code is refused
and ends the sign-in, and `demo/login.sh <engine-port> browser
[demo-ok|demo-ok:<id>|deny]` plays the browser on the worker's computer
coming back to it (`deny`: the person cancelled on the page; the
container's `BROWSER` only notes the link, so `browser: "host"` works there);
`demo/run.sh <engine-port> --login-ttl <seconds>` shortens the 10 minutes. Nothing in it reads the host's own Claude Code or Codex, and its
scripts that write files or call tmux refuse to run outside the container
(they need `/.dockerenv` and `DEMO_CONTAINER=1`, which the image sets).

## Configuration

```yaml
tmux_binary: tmux            # name on PATH or absolute path (never this worker's own binary)
socket_name: null            # tmux -L <name>; null = your default tmux server
socket_path: null            # tmux -S <path>; wins over socket_name
poll_interval_ms: 1000       # live feed cadence for sessions and agent state
ports_interval_ms: 3000      # process and listening-port re-scan cadence
busy_window_ms: 2000         # agents without working patterns: "working" while the pane changed this recently
command_timeout_ms: 5000     # one tmux command
max_terminals: 8             # concurrent Console terminals (each one is a tmux client)
terminal_idle_timeout_ms: 30000  # detach a Console terminal its page stopped heart-beating
agents:                      # first match wins; patterns are case-insensitive regexes over the pane's last lines
  - id: claude
    label: Claude Code
    process: [claude]
    working_patterns: ["esc to interrupt"]
    waiting_patterns: ["❯\\s*1\\.\\s*Yes", "Do you want to proceed\\?", "Enter to select"]
```

The built-in list also recognises `codex`, `opencode` and `gemini`; see
[`src/config.rs`](src/config.rs). Edit the configuration from the page's
settings action or with `configuration::set`.

## Observable functions

| Function | Args | Delivery | Event |
|---|---|---|---|
| `tmux::on-changed` | none; the overview has no field to filter on | iii's default delivery; once a new observer is accepted, the worker calls the function with the current overview for that observer alone (metadata `{ for_observer: <observer id> }`), delivered like every other call | `Overview { sessions, ports, usage }`: the shapes `tmux::sessions::list`, `tmux::ports::list` and `tmux::usage::read` return (`usage` is absent from workers before 0.3) |

`tmux::on-changed` is private: only this worker calls it, whenever sessions,
windows, panes, agent states, compose stacks, listening ports, Claude Code
accounts or usage readings change. The
worker polls tmux only while something observes it.

Overviews can arrive out of order: each one is its own call, and iii doesn't
order deliveries across calls, so two sent milliseconds apart (a mutation's
and the next poll's) may land the other way round, and nothing re-sends until
something changes again. Keep the overview with the highest
`sessions.generated_at_ms` (and `ports.generated_at_ms`) and drop an older one
that comes after it; the same rule settles a `list` answer racing a push.

```ts
iii.registerFunction('my-worker::on-tmux', async ({ output: { sessions, ports } }) => {
  // compare agent states with the previous overview
})
iii.observe({ function_id: 'tmux::on-changed', capture: ['output'] }, 'my-worker::on-tmux')
```

The Console page's terminal functions, `tmux::terminal::open`, `write`,
`resize`, `switch`, `replay`, `redraw`, `close` and `upload`, cannot be
observed (`NOT_OBSERVABLE`): `open` returns the terminal's access key, every
other call carries it, `write` carries what is typed into the terminal and
`upload` the file being pasted.

`tmux::terminal::upload` is how a file is pasted into a terminal. It only
writes a file: `data` (base64, any type, at most 12 MiB decoded) goes to a new
folder of its own in the cache folder of the user this worker runs as, under
`name` made safe, and the answer is that file's absolute path. It types
nothing into the terminal; the page pastes the path itself, and Claude Code and
Codex then attach it (an image as `[Image #1]`; Codex only when the paste is
that path alone and its model takes images); any other program just receives
the path as text.

```json
{ "path": "/home/me/.cache/iii/tmux/uploads/20261110-161648-0a1b2c3d/Quarterly-report.pdf",
  "bytes": 48213, "mime": "application/pdf" }
```

| Computer | Folder |
|---|---|
| Linux | `$XDG_CACHE_HOME/iii/tmux/uploads` when that's an absolute path, otherwise `~/.cache/iii/tmux/uploads` |
| macOS | `~/Library/Caches/iii/tmux/uploads` |

Each upload gets its own folder, `<UTC yyyymmdd-hhmmss>-<8 hex>`, so its name
survives. The name keeps only its base name, made one token a shell or Codex
reads whole: characters outside `A-Z a-z 0-9 . _ -` become `-` (no spaces),
runs collapse, no leading dot, at most 100 bytes with its extension kept, and
`file` when nothing is left (`../x` is `x`, `My report (1).pdf` is
`My-report-1.pdf`). A PNG, JPEG, GIF or WebP, told by its first bytes, gets the
extension its content has (`photo.HEIC` that is a JPEG is `photo.jpg`), so
agents read it as an image; `mime` in the answer is then that type, otherwise
the request's, otherwise `application/octet-stream`.

The folders are readable by this user only (0700) and so is each file (0600),
never executable. Every folder on the way below the cache folder (`iii`,
`tmux`, `uploads`) is made one at a time and has to be a real folder of this
user that no other user can write to, never a symbolic link; a folder of ours
that is wider is narrowed. A file is written whole to a temporary file and
then linked under its name, so a kill or a full disk never leaves a cut-short
file there. Uploads older than 7 days are removed when the worker starts and
on each upload, then the oldest while all of them add up to more than 256 MiB;
nothing but folders of that name pattern is touched. They are not removed
right away because Codex reads a file only when the message it is attached to
is sent.
