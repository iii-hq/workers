# provider-minimax

OpenAI-compatible provider under provider::minimax::*.

## Installation

```bash
iii worker add provider-minimax
```

## Run

```bash
iii-provider-minimax
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
