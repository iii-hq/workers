# iii-coding Worker

Worker that scaffolds new iii workers, generates functions and triggers, executes code in sandboxes, runs tests, and deploys -- all using iii primitives (worker, function, trigger).

## Functions

### coding::scaffold
Scaffolds a complete iii worker project from a definition.

**Input:**
```json
{
  "name": "my-worker",
  "language": "rust",
  "functions": [
    { "id": "my-worker::greet", "description": "Greet a user", "request_format": {}, "response_format": {} }
  ],
  "triggers": [
    { "trigger_type": "http", "function_id": "my-worker::greet", "config": { "api_path": "my-worker/greet", "http_method": "POST" } }
  ]
}
```

**Output:**
```json
{
  "worker_id": "my-worker_a1b2",
  "files": [{ "path": "src/main.rs", "content": "...", "language": "rust" }],
  "function_count": 1,
  "trigger_count": 1
}
```

### coding::generate_function
Generates a single function handler file. Optionally adds it to an existing scaffolded worker.

**Input:**
```json
{
  "worker_id": "my-worker_a1b2",
  "language": "rust",
  "id": "my-worker::compute",
  "description": "Run a computation",
  "request_format": {},
  "response_format": {}
}
```

**Output:**
```json
{
  "function_id": "fn_my_worker_compute_c3d4",
  "file_path": "src/functions/compute.rs",
  "content": "...",
  "language": "rust"
}
```

### coding::generate_trigger
Generates trigger registration code for a function.

**Input:**
```json
{
  "function_id": "my-worker::greet",
  "trigger_type": "http",
  "config": { "api_path": "my-worker/greet", "http_method": "POST" },
  "language": "rust"
}
```

**Output:**
```json
{
  "trigger_type": "http",
  "function_id": "my-worker::greet",
  "registration_code": "iii.register_trigger(...);",
  "config": { "api_path": "my-worker/greet", "http_method": "POST" }
}
```

### coding::execute
Executes code in a subprocess with timeout.

**Input:**
```json
{
  "code": "fn main() { println!(\"hello\"); }",
  "language": "rust",
  "input": {},
  "timeout_ms": 10000
}
```

**Output:**
```json
{
  "success": true,
  "stdout": "hello\n",
  "stderr": "",
  "exit_code": 0,
  "duration_ms": 1234
}
```

### coding::test
Runs tests for a scaffolded worker or inline code.

**Input (worker):**
```json
{ "worker_id": "my-worker_a1b2" }
```

**Input (inline):**
```json
{
  "code": "pub fn add(a: i32, b: i32) -> i32 { a + b }",
  "language": "rust",
  "test_code": "    #[test]\n    fn test_add() { assert_eq!(add(1, 2), 3); }"
}
```

**Output:**
```json
{
  "passed": true,
  "total": 0,
  "passed_count": 0,
  "failed_count": 0,
  "output": "..."
}
```

### coding::deploy
Returns worker files and deployment instructions.

**Input:**
```json
{ "worker_id": "my-worker_a1b2" }
```

**Output:**
```json
{
  "deployed": true,
  "worker_id": "my-worker_a1b2",
  "deployment_id": "deploy_my-worker_a1b2_e5f6",
  "files": [...],
  "instructions": "1. cd into the worker directory\n2. Run: cargo build --release\n..."
}
```

## HTTP Triggers

| Endpoint | Method | Function |
|---|---|---|
| `coding/scaffold` | POST | `coding::scaffold` |
| `coding/generate-function` | POST | `coding::generate_function` |
| `coding/generate-trigger` | POST | `coding::generate_trigger` |
| `coding/execute` | POST | `coding::execute` |
| `coding/test` | POST | `coding::test` |
| `coding/deploy` | POST | `coding::deploy` |

## State Scopes

| Scope | Key | Description |
|---|---|---|
| `coding:workers` | `{worker_id}` | Scaffolded worker definitions and files |
| `coding:functions` | `{function_id}` | Generated function code |
| `coding:deployments` | `{deployment_id}` | Deployment records |

## Supported Languages

- **Rust** -- Generates Cargo.toml, build.rs, src/main.rs, config.rs, manifest.rs, function handlers
- **TypeScript** -- Generates package.json, tsconfig.json, src/index.ts, function handlers
- **Python** -- Generates pyproject.toml, src/worker.py, function handlers

## Configuration

```yaml
workspace_dir: "/tmp/iii-coding-workspace"
supported_languages: ["rust", "typescript", "python"]
execute_timeout_ms: 30000
max_file_size_kb: 256
```
