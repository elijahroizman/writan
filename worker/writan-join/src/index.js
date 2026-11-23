// Writan 6-digit code directory for internet sharing.
//
// Maps ephemeral 6-digit join codes to a host's iroh EndpointAddr in a single
// SQLite-backed Durable Object so codes are globally unique, expire after 10
// minutes, and are rate-limited per IP. Document bytes and edits never pass
// through this Worker — it only introduces peers.

const ROOM_TTL_MS = 10 * 60 * 1000;
const RETENTION_MS = 24 * 60 * 60 * 1000;
const MISS_WINDOW_MS = 60 * 1000;
const MAX_MISSES_PER_IP = 5;
const CREATE_WINDOW_MS = 10 * 60 * 1000;
const MAX_CREATES_PER_IP = 30;
const MAX_ADDR_BYTES = 8192;

const CODE_RE = /^\d{6}$/;

function json(body, status = 200, extraHeaders = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
      ...extraHeaders,
    },
  });
}

function randomCode() {
  // Rejection-sample to avoid modulo bias on 1_000_000.
  const limit = 4_294_000_000;
  const buf = new Uint32Array(1);
  for (;;) {
    crypto.getRandomValues(buf);
    if (buf[0] < limit) {
      return String(buf[0] % 1_000_000).padStart(6, "0");
    }
  }
}

function randomSecret() {
  const bytes = new Uint8Array(24);
  crypto.getRandomValues(bytes);
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function constantTimeEqual(a, b) {
  const enc = new TextEncoder();
  const ab = enc.encode(a);
  const bb = enc.encode(b);
  if (ab.length !== bb.length) return false;
  let diff = 0;
  for (let i = 0; i < ab.length; i++) diff |= ab[i] ^ bb[i];
  return diff === 0;
}

function bearerToken(request) {
  const auth = request.headers.get("authorization") || "";
  const match = auth.match(/^Bearer\s+([A-Za-z0-9_-]{16,128})$/);
  return match ? match[1] : null;
}

function clientIp(request) {
  return (
    request.headers.get("cf-connecting-ip") ||
    request.headers.get("x-forwarded-for")?.split(",")[0]?.trim() ||
    "unknown"
  );
}

export class RoomDirectory {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
    this.sql = ctx.storage.sql;
    ctx.blockConcurrencyWhile(async () => {
      this.sql.exec(`
        CREATE TABLE IF NOT EXISTS rooms (
          code TEXT PRIMARY KEY,
          endpoint_addr TEXT NOT NULL,
          host_secret TEXT NOT NULL,
          expires_at INTEGER NOT NULL,
          created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS rate_limits (
          ip TEXT NOT NULL,
          kind TEXT NOT NULL,
          ts INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_rate_limits_ip_kind_ts
          ON rate_limits (ip, kind, ts);
      `);
    });
  }

  prune(now) {
    // Keep expired rows briefly so the host can still rotate with its hostSecret,
    // while GET /rooms/:code enforces expires_at > now.
    this.sql.exec("DELETE FROM rooms WHERE expires_at < ?", now - RETENTION_MS);
    this.sql.exec("DELETE FROM rate_limits WHERE ts < ?", now - CREATE_WINDOW_MS);
  }

  countRecent(ip, kind, since) {
    const rows = this.sql
      .exec(
        "SELECT COUNT(*) AS c FROM rate_limits WHERE ip = ? AND kind = ? AND ts >= ?",
        ip,
        kind,
        since,
      )
      .toArray();
    return Number(rows[0]?.c ?? 0);
  }

  recordEvent(ip, kind, now) {
    this.sql.exec(
      "INSERT INTO rate_limits (ip, kind, ts) VALUES (?, ?, ?)",
      ip,
      kind,
      now,
    );
  }

  allocateCode(now) {
    for (let attempt = 0; attempt < 32; attempt++) {
      const code = randomCode();
      const existing = this.sql
        .exec("SELECT expires_at FROM rooms WHERE code = ?", code)
        .toArray();
      if (existing.length === 0 || Number(existing[0].expires_at) <= now) {
        if (existing.length > 0) {
          this.sql.exec("DELETE FROM rooms WHERE code = ?", code);
        }
        return code;
      }
    }
    return null;
  }

  async fetch(request) {
    const url = new URL(request.url);
    const ip = clientIp(request);
    const now = Date.now();
    this.prune(now);

    if (request.method === "POST" && url.pathname === "/rooms") {
      if (this.countRecent(ip, "create", now - CREATE_WINDOW_MS) >= MAX_CREATES_PER_IP) {
        return json({ error: "Too many share rooms created. Try again shortly." }, 429, {
          "retry-after": "60",
        });
      }
      let payload;
      try {
        payload = await request.json();
      } catch {
        return json({ error: "Invalid JSON body." }, 400);
      }
      if (!payload || typeof payload.endpointAddr !== "object" || payload.endpointAddr === null) {
        return json({ error: "Missing endpointAddr object." }, 400);
      }
      const addrJson = JSON.stringify(payload.endpointAddr);
      if (addrJson.length > MAX_ADDR_BYTES) {
        return json({ error: "endpointAddr is too large." }, 413);
      }
      const code = this.allocateCode(now);
      if (!code) {
        return json({ error: "Could not allocate a join code. Try again." }, 503);
      }
      const hostSecret = randomSecret();
      const expiresAt = now + ROOM_TTL_MS;
      this.sql.exec(
        "INSERT INTO rooms (code, endpoint_addr, host_secret, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
        code,
        addrJson,
        hostSecret,
        expiresAt,
        now,
      );
      this.recordEvent(ip, "create", now);
      return json(
        {
          code,
          expiresAt,
          ttlSecs: Math.floor(ROOM_TTL_MS / 1000),
          hostSecret,
        },
        201,
      );
    }

    const rotateMatch = url.pathname.match(/^\/rooms\/(\d{6})\/rotate$/);
    if (request.method === "POST" && rotateMatch) {
      const oldCode = rotateMatch[1];
      const secret = bearerToken(request);
      if (!secret) return json({ error: "Missing host authorization." }, 401);

      const rows = this.sql
        .exec("SELECT endpoint_addr, host_secret FROM rooms WHERE code = ?", oldCode)
        .toArray();
      if (rows.length === 0 || !constantTimeEqual(String(rows[0].host_secret), secret)) {
        return json({ error: "Room not found or unauthorized." }, 404);
      }

      let addrJson = String(rows[0].endpoint_addr);
      try {
        const bodyText = await request.text();
        if (bodyText.trim()) {
          const body = JSON.parse(bodyText);
          if (body && typeof body.endpointAddr === "object" && body.endpointAddr !== null) {
            const candidate = JSON.stringify(body.endpointAddr);
            if (candidate.length > MAX_ADDR_BYTES) {
              return json({ error: "endpointAddr is too large." }, 413);
            }
            addrJson = candidate;
          }
        }
      } catch {
        return json({ error: "Invalid JSON body." }, 400);
      }

      const newCode = this.allocateCode(now);
      if (!newCode) {
        return json({ error: "Could not allocate a join code. Try again." }, 503);
      }
      const expiresAt = now + ROOM_TTL_MS;
      this.sql.exec("DELETE FROM rooms WHERE code = ?", oldCode);
      this.sql.exec(
        "INSERT INTO rooms (code, endpoint_addr, host_secret, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
        newCode,
        addrJson,
        secret,
        expiresAt,
        now,
      );
      return json({
        code: newCode,
        expiresAt,
        ttlSecs: Math.floor(ROOM_TTL_MS / 1000),
      });
    }

    const roomMatch = url.pathname.match(/^\/rooms\/([^/]+)$/);
    if (roomMatch) {
      const code = roomMatch[1];
      if (!CODE_RE.test(code)) {
        return json({ error: "Code must be six digits." }, 400);
      }

      if (request.method === "GET") {
        if (this.countRecent(ip, "miss", now - MISS_WINDOW_MS) >= MAX_MISSES_PER_IP) {
          return json({ error: "Too many wrong codes. Wait a minute and try again." }, 429, {
            "retry-after": "60",
          });
        }
        const rows = this.sql
          .exec(
            "SELECT endpoint_addr, expires_at FROM rooms WHERE code = ? AND expires_at > ?",
            code,
            now,
          )
          .toArray();
        if (rows.length === 0) {
          this.recordEvent(ip, "miss", now);
          return json({ error: "No active share for that code." }, 404);
        }
        return json({
          endpointAddr: JSON.parse(String(rows[0].endpoint_addr)),
          expiresAt: Number(rows[0].expires_at),
        });
      }

      if (request.method === "DELETE") {
        const secret = bearerToken(request);
        if (!secret) return json({ error: "Missing host authorization." }, 401);
        const rows = this.sql
          .exec("SELECT host_secret FROM rooms WHERE code = ?", code)
          .toArray();
        if (rows.length === 0 || !constantTimeEqual(String(rows[0].host_secret), secret)) {
          return json({ error: "Room not found or unauthorized." }, 404);
        }
        this.sql.exec("DELETE FROM rooms WHERE code = ?", code);
        return new Response(null, { status: 204 });
      }
    }

    return json({ error: "Not found." }, 404);
  }
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/health") {
      return json({ ok: true });
    }
    const id = env.ROOMS.idFromName("global");
    const stub = env.ROOMS.get(id);
    return stub.fetch(request);
  },
};
