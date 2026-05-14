# auth::token

Use this after `auth::register` to issue an access token or rotate a refresh token.

HTTP route: `POST /token`

Supported grants:

- `client_credentials`: issue a new access token and refresh token
- `refresh_token`: revoke the used refresh token, issue a new access token, and return a new refresh token

Client-secret-post input:

```json
{
  "grant_type": "client_credentials",
  "client_id": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
  "client_secret": "iN1PqXZJDEU6M5HsR3uHz12vQk1eQJ3TR1T1lPYU6Oc",
  "scope": "mcp:tools"
}
```

Client-secret-basic input:

```json
{
  "headers": {
    "authorization": "Basic UUd4R3E3bTZiY3FYa0ZZN1EwYzFwMkpmOnNlY3JldA=="
  },
  "grant_type": "client_credentials",
  "scope": "mcp:tools"
}
```

Refresh input:

```json
{
  "grant_type": "refresh_token",
  "refresh_token": "old-refresh-token",
  "client_id": "QGxGq7m6bcqXkFY7Q0c1p2Jf",
  "client_secret": "iN1PqXZJDEU6M5HsR3uHz12vQk1eQJ3TR1T1lPYU6Oc"
}
```

Sample output:

```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIsImtpZCI6...",
  "token_type": "Bearer",
  "expires_in": 900,
  "refresh_token": "QGKb6A0lHDxcnwhYE4V3pKL40ZVvL2r8G9E4jWoSvdA",
  "scope": "mcp:tools"
}
```

Never ask for wildcard scopes at token time. Wildcards can be registered for policy, but concrete tokens must carry concrete scopes.
