# auth::introspect

Use this when a trusted resource worker needs to know whether a token is active and which client/scopes it represents.

HTTP route: `POST /introspect`

Authenticate with one of:

- `client_secret_post`: include `client_id` and `client_secret` in the JSON body.
- `client_secret_basic`: send `Authorization: Basic base64(client_id:client_secret)`.

Input:

```json
{
  "client_id": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
  "client_secret": "iN1PqXZJDEU6M5HsR3uHz12vQk1eQJ3TR1T1lPYU6Oc",
  "token": "eyJhbGciOiJSUzI1NiIsImtpZCI6...",
  "token_type_hint": "access_token"
}
```

Sample active output:

```json
{
  "active": true,
  "client_id": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
  "sub": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
  "aud": "iii",
  "iss": "http://127.0.0.1:3111",
  "exp": 1770000000,
  "iat": 1769999100,
  "scope": "mcp:tools",
  "jti": "token-id"
}
```

Inactive response:

```json
{ "active": false }
```

Use `token_type_hint: refresh_token` when checking a refresh token, otherwise omit the hint or use `access_token`.

Do not expose introspection to untrusted callers. A caller with valid client credentials can learn token activity and subject data.
