# iii-coding

Instead of reading docs and writing boilerplate, describe what you want: "create a function that processes payments and expose it on POST /payments." iii-coding scaffolds the complete worker project (Cargo.toml, main.rs, function handlers, trigger wiring), executes code in sandboxes with timeouts, runs tests, and prepares deployment. It generates real, compilable iii worker code for Rust, TypeScript, or Python — following the exact same patterns as every worker in this repo.

**Plug and play:** Build with `cargo build --release`, then run `./target/release/iii-coding --url ws://your-engine:49134`. It registers 6 functions. Call `coding::scaffold` with a worker name, language, and function descriptions to generate a complete project. Call `coding::execute` to run code safely in a subprocess with timeout.

## Functions

| Function ID | Description |
|---|---|
| `coding::scaffold` | Scaffold a complete iii worker project from a definition |
| `coding::generate_function` | Generate a single function handler file |
| `coding::generate_trigger` | Generate trigger registration code for a function |
| `coding::execute` | Execute code in a subprocess with timeout |
| `coding::test` | Run tests for a scaffolded worker or inline code |
| `coding::deploy` | Return worker files and deployment instructions |

## iii Primitives Used

- **State** -- scaffolded worker definitions, generated function code, deployment records
- **HTTP** -- all functions exposed as POST endpoints

## Prerequisites

- Rust 1.75+
- Running iii engine on `ws://127.0.0.1:49134`

## Build

```bash
cargo build --release
```

## Usage

```bash
./target/release/iii-coding --url ws://127.0.0.1:49134 --config ./config.yaml
```

```
Options:
  --config <PATH>    Path to config.yaml [default: ./config.yaml]
  --url <URL>        WebSocket URL of the iii engine [default: ws://127.0.0.1:49134]
  --manifest         Output module manifest as JSON and exit
  -h, --help         Print help
```

## Configuration

```yaml
workspace_dir: "/tmp/iii-coding-workspace"          # directory for scaffolded projects
supported_languages: ["rust", "typescript", "python"] # languages for code generation
execute_timeout_ms: 30000                             # subprocess execution timeout
max_file_size_kb: 256                                 # max generated file size
```

## Tests

```bash
cargo test
```
