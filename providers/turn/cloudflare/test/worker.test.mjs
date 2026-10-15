// Contract tests for the Cloudflare TURN Worker, run with `node --test`.
// The Cloudflare API is replaced by a stub, so no network access is needed.

import assert from "node:assert/strict";
import { afterEach, beforeEach, test } from "node:test";

import worker, { browserSafe, corsHeaders, credentialTtl } from "../src/worker.js";

const ENV = {
  TURN_KEY_ID: "abc123",
  TURN_KEY_API_TOKEN: "secret-token",
  ALLOWED_ORIGINS: "https://app.example.org",
  CREDENTIAL_TTL_SECS: "600",
};

const CLOUDFLARE_RESPONSE = {
  iceServers: [
    { urls: ["stun:stun.cloudflare.com:3478", "stun:stun.cloudflare.com:53"] },
    {
      urls: [
        "turn:turn.cloudflare.com:3478?transport=udp",
        "turn:turn.cloudflare.com:53?transport=udp",
        "turns:turn.cloudflare.com:443?transport=tcp",
      ],
      username: "user",
      credential: "pass",
    },
  ],
};

let upstreamCalls;
const realFetch = globalThis.fetch;

beforeEach(() => {
  upstreamCalls = [];
  globalThis.fetch = async (url, init) => {
    upstreamCalls.push({ url, init });
    return new Response(JSON.stringify(CLOUDFLARE_RESPONSE), { status: 201 });
  };
});

afterEach(() => {
  globalThis.fetch = realFetch;
});

function get(path, headers = { origin: "https://app.example.org" }) {
  return new Request(`https://p2p-net-turn.example.workers.dev${path}`, { headers });
}

test("returns the p2p-net contract without port 53 URLs", async () => {
  const response = await worker.fetch(get("/ice-servers"), ENV);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("access-control-allow-origin"), "https://app.example.org");
  const body = await response.json();
  assert.equal(body.ttl, 600);
  assert.deepEqual(body.iceServers[1].urls, [
    "turn:turn.cloudflare.com:3478?transport=udp",
    "turns:turn.cloudflare.com:443?transport=tcp",
  ]);
  assert.equal(body.iceServers[1].credential, "pass");

  const [{ url, init }] = upstreamCalls;
  assert.equal(
    url,
    "https://rtc.live.cloudflare.com/v1/turn/keys/abc123/credentials/generate-ice-servers",
  );
  assert.equal(init.headers.authorization, "Bearer secret-token");
  assert.deepEqual(JSON.parse(init.body), { ttl: 600 });
});

test("never exposes the API token to the client", async () => {
  const response = await worker.fetch(get("/ice-servers"), ENV);
  assert.ok(!(await response.text()).includes("secret-token"));
});

test("refuses origins that are not allowed", async () => {
  const response = await worker.fetch(get("/ice-servers", { origin: "https://evil.example" }), ENV);
  assert.equal(response.status, 403);
  assert.equal(upstreamCalls.length, 0);
});

test("allows server-to-server calls without an Origin header", async () => {
  const response = await worker.fetch(get("/ice-servers", {}), ENV);
  assert.equal(response.status, 200);
});

test("answers CORS preflight and rejects other paths", async () => {
  const preflight = await worker.fetch(
    new Request("https://w.example/ice-servers", {
      method: "OPTIONS",
      headers: { origin: "https://app.example.org" },
    }),
    ENV,
  );
  assert.equal(preflight.status, 204);
  assert.equal((await worker.fetch(get("/other"), ENV)).status, 404);
});

test("reports missing configuration and upstream failures", async () => {
  const missing = await worker.fetch(get("/ice-servers"), { ...ENV, TURN_KEY_API_TOKEN: "" });
  assert.equal(missing.status, 500);

  globalThis.fetch = async () => new Response("denied", { status: 401 });
  assert.equal((await worker.fetch(get("/ice-servers"), ENV)).status, 502);
});

test("applies the optional rate limiter", async () => {
  const limited = {
    ...ENV,
    RATE_LIMITER: { limit: async () => ({ success: false }) },
  };
  assert.equal((await worker.fetch(get("/ice-servers"), limited)).status, 429);
  assert.equal(upstreamCalls.length, 0);
});

test("helpers clamp TTLs, normalize iceServers and handle wildcard CORS", () => {
  assert.equal(credentialTtl({}), 3600);
  assert.equal(credentialTtl({ CREDENTIAL_TTL_SECS: "5" }), 60);
  assert.equal(credentialTtl({ CREDENTIAL_TTL_SECS: "999999" }), 172800);
  assert.equal(credentialTtl({ CREDENTIAL_TTL_SECS: "nope" }), 3600);
  assert.deepEqual(browserSafe({ urls: "stun:s.example:53" }), []);
  assert.deepEqual(browserSafe({ urls: "stun:s.example:3478" }), [{ urls: ["stun:s.example:3478"] }]);
  const wildcard = corsHeaders(get("/ice-servers", { origin: "https://any.example" }), {
    ALLOWED_ORIGINS: "*",
  });
  assert.equal(wildcard["access-control-allow-origin"], "*");
});
