# provider-google

Google Gemini provider under provider::google::*.

## Installation

```bash
iii worker add provider-google
```

## Run

```bash
iii-provider-google
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
