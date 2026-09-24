# auth::jwks

Use this when a verifier needs public keys for RS256 access tokens issued by this auth worker.

HTTP route: `GET /.well-known/jwks.json`

Input:

```json
{}
```

Sample output:

```json
{
  "keys": [
    {
      "kty": "RSA",
      "use": "sig",
      "kid": "current-key-id",
      "alg": "RS256",
      "n": "base64url-modulus",
      "e": "AQAB"
    }
  ]
}
```

Cache by `kid`, but refresh this document when token validation sees an unknown `kid`. After `auth::jwks_rotate`, the previous key remains available until the configured overlap window ends.
