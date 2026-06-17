# lsp-vscode Agent Notes

Use containers for Node commands. Do not install host-global Node packages.

```bash
cd lsp-vscode
docker build -t lsp-vscode-dev .
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/workspace" -w /workspace lsp-vscode-dev npm ci
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/workspace" -w /workspace lsp-vscode-dev npm test
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/workspace" -w /workspace lsp-vscode-dev npm run package:check
```
