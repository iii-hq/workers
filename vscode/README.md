# vscode

VS Code as an iii worker. The worker runs the VS Code Server through the `code` CLI (`code serve-web`), owns each process and its isolated profile, and ships a **VS Code** Console page that embeds the Workbench for the chat's working directory. Nothing is re-implemented: the Console page is the same web Workbench the CLI serves, with the same extensions, settings sync, and terminals.

## Install

```bash
iii worker add vscode
```

Requires the VS Code CLI (`code`) on the host. Install VS Code and enable the shell command, or download the standalone CLI and point `VSCODE_SERVER_BIN` at it. The first start of a workspace downloads the matching VS Code Server build into the worker's data directory; starting the worker accepts the [VS Code Server license terms](https://aka.ms/vscode-server-license).

## Configuration

Environment variables, all optional:

| Variable | Default | Purpose |
| --- | --- | --- |
| `VSCODE_SERVER_BIN` | `code` | Path to the VS Code CLI |
| `VSCODE_DATA_DIR` | `~/.iii/vscode` | Per-workspace server and CLI data directories |
| `VSCODE_BIND_HOST` | `127.0.0.1` | Loopback address the server listens on |
| `VSCODE_PORT_MIN` / `VSCODE_PORT_MAX` | `18080` / `18180` | Port range, one port per running workspace |
| `VSCODE_START_TIMEOUT_MS` | `180000` | How long a start waits for the server to answer |
| `III_URL` | `ws://127.0.0.1:49134` | Engine address |

Each workspace runs as:

```text
code --cli-data-dir <data>/<id>/cli-data serve-web \
  --host 127.0.0.1 --port <free port> \
  --without-connection-token \
  --accept-server-license-terms \
  --server-data-dir <data>/<id>/server-data \
  --default-folder <workspace> \
  --disable-telemetry
```

## Console

Open **VS Code** from the Console navigation or the command palette (`Open VS Code`). The page follows the chat's working directory and starts a Workbench for it; when no directory is set it offers the Console's recent folders. Page commands: `R` reload the Workbench, `O` open it in a browser tab, `X` stop the server. Another worker can open a specific folder with `host.panels.open({ pageId: 'vscode', context: { workspace: '/path' } })`.

The Workbench renders inside an iframe because VS Code Web is a complete application with its own service worker and origin, not a React component. Keys typed inside the Workbench stay with VS Code; the Console's page commands answer while focus is on the page chrome.

## Functions

| Function | Purpose |
| --- | --- |
| `vscode::start` | Start a Workbench for an absolute `workspace` directory, or return the running one. `id` defaults to a stable hash of the path; `name` is a label. |
| `vscode::instances::list` | Every process the worker owns, with `host`, `port`, `pid`, `status`, and `exit_code`. |
| `vscode::stop` | Stop a process group by `id`. |
| `vscode::delete` | Stop a process and forget it; `delete_profile: true` also removes its data directory. |

`vscode::start` and `vscode::instances::list` are pre-approved in `iii-permissions.yaml`; `stop` and `delete` remain approval-gated.

## Security

Browsers drop the VS Code Server connection-token cookie inside a cross-origin Console iframe, so the worker runs the server in cookie-free mode and refuses to bind anywhere but loopback (`127.0.0.1`, `localhost`, `::1`). The worker exits at boot if `VSCODE_BIND_HOST` is set to anything else.

The server is meant for a local single-user Console. Remote or multi-user deployments need a same-origin authenticated proxy in front of it before relaxing this.

## Development

```bash
pnpm install          # in the repository root, for the ui workspace
cd vscode && pnpm install
pnpm build            # ui bundle + single-file worker bundle
pnpm test
pnpm lint
```

Set `III_VSCODE_UI_WATCH=1` to serve the page from `ui/dist` and hot-reload it into open Console tabs while `pnpm --dir ui watch` runs.
