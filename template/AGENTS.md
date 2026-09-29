# Template tooling

Use the container for Python dependencies and validation. From the repository root:

```sh
docker build -t workers-template-tests template
docker run --rm --network none --user "$(id -u):$(id -g)" \
  -e PYTHONDONTWRITEBYTECODE=1 -v "$PWD:/workspace:ro" \
  workers-template-tests
docker run --rm --network none -v "$PWD:/workspace:ro" \
  workers-template-tests bash -n template/sync.sh
```

Tests use temporary local Git repositories. Do not start the worker stack for
download or Compose rewrite tests. Rust workers run directly with Cargo.
