# iii-lsp-vscode Agent Notes

Use containers for Node commands. Do not install host-global Node packages.

```bash
cd iii-lsp-vscode
docker build -t iii-lsp-vscode-dev .
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/workspace" -w /workspace iii-lsp-vscode-dev npm ci
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/workspace" -w /workspace iii-lsp-vscode-dev npm test
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/workspace" -w /workspace iii-lsp-vscode-dev npm run package:check
```
