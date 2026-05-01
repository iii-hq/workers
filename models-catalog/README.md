# models-catalog

Model capabilities knowledge base on the iii bus. The bus is the source of
truth: models live under `models:<provider>:<id>` in scope `models`. The
embedded `data/models.json` is used only as a one-time seed when state is
empty; subsequent registrations win.

## Installation

```bash
iii worker add models-catalog
```

## Run

```bash
iii-models-catalog --engine-url ws://127.0.0.1:49134
```

## Registered functions

| Function | Description |
|---|---|
| `models::list` | Read all known models from state. |
| `models::get` | Read a model by `provider` + `id`. |
| `models::supports` | Capability lookup (transport, thinking, cache retention). |
| `models::register` | Write a model entry to state. |

## Build

```bash
cargo build --release
```
