---
name: vscode
description: >-
  Run the VS Code Server for a workspace directory and open the Workbench in
  the Console; use it when a person needs a full editor next to the chat, not
  when the agent itself needs to read or edit files.
---

# vscode

The vscode worker runs the VS Code Server through the `code` CLI, one loopback
process per workspace directory with its own isolated profile, and ships a
Console page that embeds the Workbench for the chat's working directory. It
gives people the editor they already use, with their extensions, settings sync,
and terminals, inside the Console beside the conversation. Requires the VS Code
CLI on the host; the first start of a workspace downloads the matching server
build.

The worker owns process lifecycle only. It does not read, write, or search
files on the agent's behalf and it never exposes the server beyond loopback.

## When to Use

- A person asks to open the working directory, a repository, or a folder in
  VS Code, or wants an editor beside the chat.
- A person wants to inspect a change in a real editor before continuing the
  conversation, and `vscode::start` for that folder returns a running server
  the Console page then shows.
- A workspace's server should be reused, listed, stopped, or its profile
  removed after work is done.

## Boundaries

- Not a file API. For reading, searching, or editing files use `shell` or
  `coder::*`; for a shared buffer the agent and person both see, use `editor`.
- Not remote access. The server binds only to loopback and refuses any other
  `bind_host`; a remote or multi-user Console needs its own authenticated
  proxy first.
- Starting a server exposes the host filesystem to the local browser, so
  `vscode::start` is allowed by default while `vscode::stop` and
  `vscode::delete` stay approval-gated.
- The Workbench itself is VS Code Web; the worker adds no editor features of
  its own.

## Functions

- `vscode::start` — start a server for an absolute `workspace` directory, or return the running one; the id defaults to a stable hash of the path.
- `vscode::instances::list` — every server the worker owns with host, port, pid, status, and exit code.
- `vscode::stop` — stop a server's process group by id.
- `vscode::delete` — stop a server, forget it, and optionally remove its data directory with `delete_profile`.

The Console page calls `vscode::start` for the chat's working directory and
opens `http://<host>:<port>/` in a frame; another worker can open a specific
folder with `panels.open({ pageId: 'vscode', context: { workspace } })`.
Configuration (CLI path, data directory, bind host, port range, timeouts) is
managed by the `configuration` worker under the id `vscode`.
