// Writan 6-digit code directory for internet sharing.
//
// Maps six-digit join codes to a host's iroh address and session id, in one
// SQLite-backed Durable Object, so codes are unique, last ten minutes, and are
// rate limited per address. Documents and edits never pass through here; it only
// introduces the apps to each other.
//
// Every request is handled without awaiting once it has its body, so a Durable
// Object's one-request-at-a-time guarantee makes each check and change atomic.

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

// Who's asking, for rate limits: the address Cloudflare saw. Over IPv6 one
// person usually has a whole /64, so that counts as one.
function clientKey(request) {
  const ip = request.headers.get("cf-connecting-ip") || "unknown";
  if (!ip.includes(":")) return ip;
  const [head, tail] = ip.toLowerCase().split("::");
  const h = head ? head.split(":") : [];
  const t = tail ? tail.split(":") : [];
  const groups = tail === undefined ? h : [...h, ...Array(Math.max(0, 8 - h.length - t.length)).fill("0"), ...t];
  return groups.slice(0, 4).map((g) => g.replace(/^0+(?=.)/, "")).join(":") + "::/64";
}

// The request's `endpointAddr`, as JSON text: `{ addr }`, or `{ error }` as a
// Response. With `optional`, an empty body gives `{ addr: null }`.
async function readAddr(request, optional = false) {
  const text = await request.text();
  if (text.length > MAX_ADDR_BYTES * 2) return { error: json({ error: "Request too large." }, 413) };
  if (optional && !text.trim()) return { addr: null };
  let payload;
  try {
    payload = JSON.parse(text);
  } catch {
    return { error: json({ error: "Invalid JSON body." }, 400) };
  }
  if (!payload || typeof payload.endpointAddr !== "object" || payload.endpointAddr === null) {
    return { error: json({ error: "Missing endpointAddr object." }, 400) };
  }
  const addr = JSON.stringify(payload.endpointAddr);
  if (addr.length > MAX_ADDR_BYTES) return { error: json({ error: "endpointAddr is too large." }, 413) };
  return { addr };
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

  allocateCode(now, except = null) {
    for (let attempt = 0; attempt < 32; attempt++) {
      const code = randomCode();
      if (code === except) continue;
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
    const ip = clientKey(request);

    if (request.method === "POST" && url.pathname === "/rooms") {
      const body = await readAddr(request);
      if (body.error) return body.error;
      const now = Date.now();
      this.prune(now);
      if (this.countRecent(ip, "create", now - CREATE_WINDOW_MS) >= MAX_CREATES_PER_IP) {
        return json({ error: "Too many share rooms created. Try again shortly." }, 429, {
          "retry-after": "60",
        });
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
        body.addr,
        hostSecret,
        expiresAt,
        now,
      );
      this.recordEvent(ip, "create", now);
      return json({ code, expiresAt, ttlSecs: ROOM_TTL_MS / 1000, hostSecret }, 201);
    }

    const rotateMatch = url.pathname.match(/^\/rooms\/(\d{6})\/rotate$/);
    if (request.method === "POST" && rotateMatch) {
      const oldCode = rotateMatch[1];
      const secret = bearerToken(request);
      if (!secret) return json({ error: "Missing host authorization." }, 401);
      const body = await readAddr(request, true);
      if (body.error) return body.error;
      const now = Date.now();
      this.prune(now);
      const rows = this.sql
        .exec("SELECT endpoint_addr, host_secret FROM rooms WHERE code = ?", oldCode)
        .toArray();
      if (rows.length === 0 || !constantTimeEqual(String(rows[0].host_secret), secret)) {
        return json({ error: "Room not found or unauthorized." }, 404);
      }
      const newCode = this.allocateCode(now, oldCode);
      if (!newCode) {
        return json({ error: "Could not allocate a join code. Try again." }, 503);
      }
      const expiresAt = now + ROOM_TTL_MS;
      this.sql.exec("DELETE FROM rooms WHERE code = ?", oldCode);
      this.sql.exec(
        "INSERT INTO rooms (code, endpoint_addr, host_secret, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
        newCode,
        body.addr ?? String(rows[0].endpoint_addr),
        secret,
        expiresAt,
        now,
      );
      return json({ code: newCode, expiresAt, ttlSecs: ROOM_TTL_MS / 1000 });
    }

    const roomMatch = url.pathname.match(/^\/rooms\/([^/]+)$/);
    if (roomMatch) {
      const code = roomMatch[1];
      if (!CODE_RE.test(code)) {
        return json({ error: "Code must be six digits." }, 400);
      }
      const now = Date.now();
      this.prune(now);

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
    // Too big to be an address: turned away here, before the Durable Object reads it.
    if (Number(request.headers.get("content-length") || 0) > MAX_ADDR_BYTES * 2) {
      return json({ error: "Request too large." }, 413);
    }
    const id = env.ROOMS.idFromName("global");
    const stub = env.ROOMS.get(id);
    return stub.fetch(request);
  },
};
