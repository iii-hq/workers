# provider-bedrock

AWS Bedrock provider under provider::bedrock::*. (stub today; emits a not-implemented error.)

## Installation

```bash
iii worker add provider-bedrock
```

## Run

```bash
iii-provider-bedrock
```

## Worker dependencies

| Worker | Range | Reason |
|---|---|---|
| `auth-credentials` | `^0.1.0` | Provider fetches its API key via `auth::get_token`. |

## Build

```bash
cargo build --release
```
