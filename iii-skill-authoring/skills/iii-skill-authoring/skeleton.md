# Worker skeleton

Copy-paste starter for a new worker. After cloning, every placeholder is one `s/<worker>/your-name/` away from a working partial set. Run `iii-skill-check render --write <worker>` once the placeholders are filled in.

## File layout to create

```
<worker>/
├── iii.worker.yaml
├── config.yaml
├── Cargo.toml             # or the manifest for your language
└── docs/
    ├── intro.md
    ├── quickstart.md
    └── leaves/
        └── <verb>.md
```

Optional partials:

- `docs/companions.md` — when this worker pairs with a sibling.
- `docs/migration.md` — when there is a breaking change to flag.

## `iii.worker.yaml`

```yaml
iii: v1
name: <worker>
language: rust
deploy: binary
manifest: Cargo.toml
bin: <worker>
description: One-sentence description that ends up in the skills index.
```

## `config.yaml`

```yaml
# <worker> runtime config.

# One-line comment per field. The renderer inlines this verbatim under ## Configuration.
key: value
```

## `docs/intro.md`

```markdown
One paragraph: what the worker does, who calls it, and the single most important thing it gives you.

<!-- llm-only:start -->
Optional second paragraph that LLM agents see but human readers do not.
<!-- llm-only:end -->
```

## `docs/quickstart.md`

Use a four-backtick outer fence when the partial contains its own fenced code block (the example below contains a triple-backtick `rust` block):

````markdown
```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let result = iii
        .trigger(TriggerRequest {
            function_id: "<worker>::<verb>".into(),
            payload: json!({ /* realistic input */ }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

The example calls `<worker>::<verb>`. Other entry points: `<worker>::<verb-2>`, `<worker>::<verb-3>`.
````

## `docs/leaves/<verb>.md`

```markdown
# Topical phrase, not the function id

## When to use

- One bullet per realistic call site.

## Notes

- Gotchas, edge cases, behaviour an agent will trip on.
```

## `docs/companions.md` (optional)

Four-backtick outer fence again — the body has its own `bash` fence:

````markdown
To <do something specific> with this worker, add the [<sibling>](../<sibling>) worker as well:

```bash
iii worker add <sibling>
```
````

## After filling in placeholders

```bash
cargo run --manifest-path iii-skill-check/Cargo.toml -- render --write <worker>
cargo run --manifest-path iii-skill-check/Cargo.toml -- verify <worker> --layers structure,vale
```

The first command produces `<worker>/README.md`, `<worker>/skill.md`, and `<worker>/skills/*.md`. The second exercises Layer 1 (structure) and Layer 2 (Vale) locally. The AI layer requires `ANTHROPIC_API_KEY`; CI runs it on every PR.

## Worked example

`workers/textstats/` is the canonical fixture worker — three functions, one llm-only block, every renderer slot exercised. Read its partials under `textstats/docs/` to see what the placeholders look like filled in.
