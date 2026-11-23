# writan-join

Tiny Cloudflare Worker + SQLite Durable Object that maps ephemeral 6-digit join codes to a host's `iroh` `EndpointAddr` for Writan's internet sharing.

No document content, edit operations, or encryption keys pass through this Worker — it only introduces peers.

## Endpoints

- `GET /health` — Health check (`{"ok":true}`)
- `POST /rooms` — Body `{"endpointAddr": ...}` → `201 {"code","expiresAt","ttlSecs","hostSecret"}`
- `GET /rooms/<code>` — Per-IP rate-limited (5 misses/min) lookup → `200 {"endpointAddr","expiresAt"}`
- `POST /rooms/<code>/rotate` — `Authorization: Bearer <hostSecret>` → allocates a fresh 6-digit code and 10-minute TTL
- `DELETE /rooms/<code>` — `Authorization: Bearer <hostSecret>` → removes the code immediately

## Self-hosting / Redeploying

Requires a free Cloudflare account (Workers Free supports SQLite Durable Objects):

```bash
cd worker/writan-join
npx wrangler deploy
```

Override the directory URL in Writan at runtime via `WRITAN_JOIN_URL` or at build time via `WRITAN_JOIN_URL`.
