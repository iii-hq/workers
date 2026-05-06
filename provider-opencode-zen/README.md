# provider-opencode-zen

OpenAI-compatible provider under provider::opencode-zen::*.

## Installation

```bash
iii worker add provider-opencode-zen
```

## Run

```bash
iii-provider-opencode-zen
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
