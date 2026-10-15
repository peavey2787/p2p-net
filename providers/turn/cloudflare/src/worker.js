// p2p-net TURN credential endpoint for Cloudflare Realtime TURN.
//
// GET /ice-servers returns the p2p-net provider contract
//   { "iceServers": [...], "ttl": <seconds> }
// so browsers can use Cloudflare TURN through `{"type": "http"}` without ever
// seeing the TURN key's API token, which stays in this Worker as a secret.

const CLOUDFLARE_API = "https://rtc.live.cloudflare.com";
const DEFAULT_TTL_SECS = 3600;
const MIN_TTL_SECS = 60;
const MAX_TTL_SECS = 48 * 3600;
// Browsers block port 53; Cloudflare recommends dropping those URLs.
const BROWSER_BLOCKED_PORT = ":53";

export default {
  async fetch(request, env) {
    const cors = corsHeaders(request, env);
    if (cors === null) {
      return json({ error: "origin not allowed" }, 403, {});
    }
    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: cors });
    }
    const { pathname } = new URL(request.url);
    if (request.method !== "GET" || pathname !== "/ice-servers") {
      return json({ error: "not found" }, 404, cors);
    }
    if (!env.TURN_KEY_ID || !env.TURN_KEY_API_TOKEN) {
      return json({ error: "TURN_KEY_ID and TURN_KEY_API_TOKEN must be configured" }, 500, cors);
    }
    if (env.RATE_LIMITER) {
      const client = request.headers.get("cf-connecting-ip") || "unknown";
      const { success } = await env.RATE_LIMITER.limit({ key: client });
      if (!success) {
        return json({ error: "rate limited" }, 429, cors);
      }
    }

    const ttl = credentialTtl(env);
    const upstream = await fetch(
      `${CLOUDFLARE_API}/v1/turn/keys/${encodeURIComponent(env.TURN_KEY_ID)}/credentials/generate-ice-servers`,
      {
        method: "POST",
        headers: {
          authorization: `Bearer ${env.TURN_KEY_API_TOKEN}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({ ttl }),
      },
    );
    if (!upstream.ok) {
      return json({ error: `Cloudflare TURN API returned ${upstream.status}` }, 502, cors);
    }
    const body = await upstream.json();
    return json({ iceServers: browserSafe(body.iceServers), ttl }, 200, {
      ...cors,
      "cache-control": "no-store",
    });
  },
};

// `iceServers` with port-53 URLs removed and empty entries dropped.
export function browserSafe(iceServers) {
  const servers = Array.isArray(iceServers) ? iceServers : [iceServers];
  return servers
    .filter((server) => server && server.urls)
    .map((server) => ({
      ...server,
      urls: [].concat(server.urls).filter((url) => !url.split("?")[0].endsWith(BROWSER_BLOCKED_PORT)),
    }))
    .filter((server) => server.urls.length > 0);
}

export function credentialTtl(env) {
  const ttl = Number.parseInt(env.CREDENTIAL_TTL_SECS ?? `${DEFAULT_TTL_SECS}`, 10);
  if (!Number.isFinite(ttl)) {
    return DEFAULT_TTL_SECS;
  }
  return Math.min(Math.max(ttl, MIN_TTL_SECS), MAX_TTL_SECS);
}

// CORS headers for an allowed origin, or null when the origin is refused.
// `ALLOWED_ORIGINS` is a comma-separated list; `*` allows every origin.
export function corsHeaders(request, env) {
  const allowed = (env.ALLOWED_ORIGINS ?? "")
    .split(",")
    .map((origin) => origin.trim())
    .filter(Boolean);
  const origin = request.headers.get("origin");
  const headers = {
    "access-control-allow-methods": "GET, OPTIONS",
    "access-control-max-age": "86400",
    vary: "origin",
  };
  if (allowed.includes("*")) {
    return { ...headers, "access-control-allow-origin": "*" };
  }
  if (origin === null) {
    // Not a browser request (no Origin header): a p2p-net broker node or curl.
    return headers;
  }
  if (!allowed.includes(origin)) {
    return null;
  }
  return { ...headers, "access-control-allow-origin": origin };
}

function json(body, status, headers) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...headers, "content-type": "application/json" },
  });
}
