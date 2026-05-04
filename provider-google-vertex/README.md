# provider-google-vertex

Vertex AI Gemini provider under provider::google-vertex::*.

## Installation

```bash
iii worker add provider-google-vertex
```

## Run

```bash
iii-provider-google-vertex
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
