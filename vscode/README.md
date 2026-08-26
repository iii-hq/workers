# vscode

Run the actual Microsoft Visual Studio Code Workbench as a standalone iii worker. The worker owns the official VS Code Server process (`code serve-web`), isolated server data, workspace lifecycle, and an injectable **VS Code** Console page containing only the Workbench.

## Runtime requirement

Install the official Microsoft VS Code CLI and make `code` available on `PATH`, or configure its absolute path:

```yaml
VSCODE_SERVER_BIN: /path/to/code
VSCODE_DATA_DIR: ~/.iii/vscode
VSCODE_BIND_HOST: 127.0.0.1
VSCODE_PORT_MIN: 18080
VSCODE_PORT_MAX: 18180
```

The worker invokes the official CLI as follows:

```text
code --cli-data-dir <isolated> serve-web \
  --host 127.0.0.1 \
  --without-connection-token \
  --server-data-dir <isolated> \
  --default-folder <console-working-directory> \
  --accept-server-license-terms \
  --disable-telemetry
```

On first launch, the CLI downloads the matching VS Code Server and Workbench build. Starting the worker accepts the [VS Code Server license terms](https://aka.ms/vscode-server-license).

## Console

Open **VS Code** from Console navigation. The active Console working directory opens automatically in the actual Microsoft Workbench. There is no separate instance manager or replacement editor UI.

The Workbench runs in its own document inside the injected Console page because VS Code Web is a complete application rather than an injectable React component. The worker does not call Browser, Shell, or another worker.

## Security

Browsers block VS Code Server's connection-token cookie in a cross-origin Console iframe. The worker therefore uses the official cookie-free mode and strictly restricts it to loopback. Startup fails if `VSCODE_BIND_HOST` is not `127.0.0.1`, `localhost`, or `::1`.

The server is intended for a local, single-user Console. Remote or multi-user deployments require a same-origin authenticated Console proxy before relaxing this restriction.

## Functions

- `vscode::start` starts or reuses a Workbench for an absolute workspace.
- `vscode::instances::list` reports worker-owned processes without URLs.
- `vscode::stop` stops the full VS Code Server process group.
- `vscode::delete` stops a process and optionally removes its isolated profile.
