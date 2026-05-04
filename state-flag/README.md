# state-flag

Per-session boolean flags on the iii bus. Backed by iii state at
`session/<id>/<name>`.

## Installation

```bash
iii worker add state-flag
```

## Run

```bash
iii-state-flag --engine-url ws://127.0.0.1:49134
```

(Or set `III_URL`.)

## Registered functions

| Function | Description |
|---|---|
| `flag::set` | Set `{ session_id, name }` to true. |
| `flag::clear` | Clear `{ session_id, name }` (set to false / absent). |
| `flag::is_set` | Read `{ session_id, name }` → boolean. |

## Build

```bash
cargo build --release
```
