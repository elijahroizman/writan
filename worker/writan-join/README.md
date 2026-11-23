# writan-join

Tiny Cloudflare Worker + SQLite Durable Object that maps ephemeral 6-digit join codes to a host's `iroh` `EndpointAddr` for Writan's internet sharing.

No document content, edit operations, or encryption keys pass through this Worker — it only introduces peers.

## Endpoints

- `GET /health` — Health check (`{"ok":true}`)
- `POST /rooms` — Body `{"endpointAddr": ...}` → `201 {"code","expiresAt","ttlSecs","hostSecret"}`
- `GET /rooms/<code>` — lookup, 5 misses a minute per address (an IPv6 /64 counts as one) → `200 {"endpointAddr","expiresAt"}`
- `POST /rooms/<code>/rotate` — `Authorization: Bearer <hostSecret>` → allocates a fresh 6-digit code and 10-minute TTL
- `DELETE /rooms/<code>` — `Authorization: Bearer <hostSecret>` → removes the code immediately

## Self-hosting / Redeploying

Requires a free Cloudflare account (Workers Free supports SQLite Durable Objects):

```bash
cd worker/writan-join
npx wrangler deploy
```

`WRITAN_JOIN_URL`, set when Writan runs or when it's built, points the app at another directory.
