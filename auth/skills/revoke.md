# auth::revoke

Use this when a client signs out, a refresh token is rotated out of use, or an operator needs an access token or refresh token to stop working.

HTTP route: `POST /revoke`

Authenticate with `client_secret_post` or `client_secret_basic`.

Input:

```json
{
  "client_id": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
  "client_secret": "iN1PqXZJDEU6M5HsR3uHz12vQk1eQJ3TR1T1lPYU6Oc",
  "token": "eyJhbGciOiJSUzI1NiIsImtpZCI6...",
  "token_type_hint": "access_token"
}
```

Sample output:

```json
{
  "ok": true
}
```

Use `token_type_hint: refresh_token` for refresh tokens. Unknown tokens also return success so callers cannot use revocation as a token oracle.
