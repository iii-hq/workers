# provider-fireworks

OpenAI-compatible provider under provider::fireworks::*.

## Installation

```bash
iii worker add provider-fireworks
```

## Run

```bash
iii-provider-fireworks
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
