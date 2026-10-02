'use strict';

// Browser relay-assisted direct upgrade QA (real Chromium, real relay).
//
// Scenarios:
//  1. browser <-> browser: relayed connection -> WebRTC signaling over it ->
//     direct `/webrtc` -> relayed connection closed; afterwards application
//     traffic flows while the relay's metered transit stays frozen.
//  2. browser <-> browser with direct negotiation forced to fail (ICE policy
//     `relay` with no TURN server): the relay stays the path and traffic keeps
//     flowing through it (relay transit grows).
//  3. browser -> native over WebRTC-direct (direct dial, no relay transit).
//  4. browser -> native relay-only peer: the native peer does not speak
//     browser `/webrtc` signaling, so the relay is the (working) fallback.
//  5. no LAN dependency: LAN discovery, DHT, rendezvous and public fallback are
//     all disabled; the direct path exists only because signaling ran over the
//     relayed connection.
//
// Environment: P2P_WASM_PKG_DIR (wasm-pack --target web output) and
// P2P_QA_RELAY_BIN (the built `qa_browser_relay` example).
//
// Scope note: both browsers run on one host, so ICE uses host candidates.
// Reaching genuinely remote/CGNAT browsers additionally needs server-reflexive
// candidates from a STUN server the operator runs (e.g. on its relays); no
// public STUN/TURN service is used here or by default.

const { spawn } = require('node:child_process');
const readline = require('node:readline');
const { chromium } = require('playwright');
const { STEP_TIMEOUT_MS, packageRoot, servePackage, waitForWasm, withTimeout } = require('./support.cjs');

const TOPIC_TIMEOUT_MS = 30000;
const MESSAGES = 8;

function log(line) {
  process.stdout.write(`${line}\n`);
}

function startHarness() {
  const bin = process.env.P2P_QA_RELAY_BIN;
  if (!bin) throw new Error('P2P_QA_RELAY_BIN is required');
  const child = spawn(bin, [], { stdio: ['pipe', 'pipe', 'inherit'] });
  const lines = readline.createInterface({ input: child.stdout });
  const queue = [];
  const waiters = [];
  lines.on('line', line => {
    const text = line.trim();
    if (!text.startsWith('{')) return;
    const value = JSON.parse(text);
    const waiter = waiters.shift();
    if (waiter) waiter(value);
    else queue.push(value);
  });
  const next = () =>
    queue.length ? Promise.resolve(queue.shift()) : new Promise(resolve => waiters.push(resolve));
  return {
    child,
    next,
    async stats() {
      child.stdin.write('stats\n');
      return withTimeout('harness stats', next());
    },
    stop() {
      child.stdin.write('quit\n');
      child.stdin.end();
    },
  };
}

function nodeConfig(relayAddr, { relayOnlyIce = false } = {}) {
  return {
    relay_peers: [relayAddr],
    reserve_configured_relays: true,
    heartbeat_interval_secs: 1,
    public_ip_probe: { enabled: false },
    discovery: {
      lan: { enabled: false },
      public_bootstrap: { mode: 'disabled', auto_connect_discovered_peers: false },
      dht: { enabled: false, announce: false, discover: false },
      rendezvous: { client_enabled: false, server_enabled: false },
    },
    browser_webrtc: { ice_servers: [], ice_transport_policy: relayOnlyIce ? 'relay' : 'all' },
  };
}

async function openNode(browser, baseUrl, label, config) {
  const context = await browser.newContext();
  const page = await context.newPage();
  page.setDefaultTimeout(STEP_TIMEOUT_MS);
  const errors = [];
  page.on('pageerror', error => errors.push(String(error && error.stack ? error.stack : error)));
  const consoleLines = [];
  page.on('console', message => consoleLines.push(`${message.type()}: ${message.text()}`));
  await page.goto(baseUrl, { waitUntil: 'load' });
  await waitForWasm(page, label);
  const peerId = await withTimeout(
    `${label}: start node`,
    page.evaluate(async ({ cfg, ns }) => {
      window.__node = await window.__p2pNet.WasmNode.start(cfg, ns);
      window.__inbox = [];
      window.__events = [];
      const events = window.__node.subscribeEvents();
      (async () => {
        for (;;) {
          const event = await events.recv();
          window.__events.push(`${Date.now() % 100000} ${JSON.stringify(event)}`);
        }
      })();
      return window.__node.peerId();
    }, { cfg: config, ns: `qa-direct-${label}-${Date.now()}` }),
  );
  return { label, context, page, peerId, errors, consoleLines };
}

// The node keeps only its newest 24 pulses, so heartbeats can push a
// diagnostic tag out between polls. Every snapshot folds its pulses into a
// per-node log that the assertions read instead.
async function snapshot(node) {
  const snap = await node.page.evaluate(async () => JSON.parse(await window.__node.snapshot()));
  node.pulseLog = node.pulseLog || new Set();
  for (const line of snap.pulses) node.pulseLog.add(line);
  return snap;
}

const sawPulse = (node, tag) => [...(node.pulseLog || [])].some(line => line.startsWith(tag));

async function waitFor(label, predicate, timeoutMs = STEP_TIMEOUT_MS) {
  const started = Date.now();
  for (;;) {
    const value = await predicate();
    if (value) return value;
    if (Date.now() - started > timeoutMs) throw new Error(`${label} timed out after ${timeoutMs} ms`);
    await new Promise(resolve => setTimeout(resolve, 250));
  }
}


async function subscribe(node, topic) {
  await node.page.evaluate(async t => {
    const subscription = await window.__node.subscribe(t);
    (async () => {
      for (;;) {
        const message = await subscription.recv();
        window.__inbox.push(message);
      }
    })();
  }, topic);
}

async function sendUntilDelivered(from, toPeerId, topic, count) {
  await waitFor(`${from.label}: topic peers`, () =>
    from.page.evaluate(async ({ peer, t }) => {
      try {
        await window.__node.sendMessage(peer, t, new Uint8Array(0));
        return true;
      } catch (_) {
        return false;
      }
    }, { peer: toPeerId, t: topic }), TOPIC_TIMEOUT_MS);
  for (let i = 0; i < count; i += 1) {
    const error = await from.page.evaluate(async ({ peer, t, i }) => {
      try {
        await window.__node.sendMessage(peer, t, new Uint8Array(4096).fill(i));
        return null;
      } catch (e) {
        return `${e && e.kind ? e.kind : ''}: ${e && e.message ? e.message : JSON.stringify(e)}`;
      }
    }, { peer: toPeerId, t: topic, i });
    if (error) throw new Error(`${from.label}: send ${i} to ${toPeerId} failed: ${error}`);
  }
}

const inboxSize = node => node.page.evaluate(() => window.__inbox.length);

// Relay aggregates refresh on the relay's 1 s observability tick.
const RELAY_TICK_MS = 1500;
const settle = () => new Promise(resolve => setTimeout(resolve, RELAY_TICK_MS));

async function relayBytesGrewPast(harness, before, label) {
  return waitFor(`${label}: relay transit grows`, async () => {
    const stats = await harness.stats();
    return stats.relayBytes > before.relayBytes && stats;
  }, 15000);
}

async function browserPair(browser, baseUrl, info, options, harness) {
  const a = await openNode(browser, baseUrl, `${options.name}-a`, nodeConfig(info.relay.addr, options));
  const b = await openNode(browser, baseUrl, `${options.name}-b`, nodeConfig(info.relay.addr, options));
  await waitFor(`${options.name}: both reserved`, async () =>
    (await snapshot(a)).relay_client_reservations > 0 && (await snapshot(b)).relay_client_reservations > 0)
    .catch(async error => {
      for (const node of [a, b]) {
        const snap = await snapshot(node);
        const pulses = snap.pulses.filter(line => !line.includes('heartbeat'));
        log(`DIAG ${node.label}: reservations=${snap.relay_client_reservations} attempts=${snap.relay_client_reservation_attempts} failures=${snap.relay_client_reservation_failures} transports=${JSON.stringify(snap.active_transports)} pulses=${JSON.stringify([...(node.pulseLog || [])].filter(line => !line.includes('heartbeat')).slice(-25))}`);
        log(`DIAG ${node.label} events: ${JSON.stringify(await node.page.evaluate(() => window.__events.filter(e => !e.includes('local_binding')).slice(-20)))}`);
      }
      log(`DIAG relay: ${JSON.stringify(await harness.stats())}`);
      throw error;
    });
  await subscribe(a, info.topic);
  await subscribe(b, info.topic);
  const circuit = `${info.relay.addr}/p2p-circuit/p2p/${b.peerId}`;
  await a.page.evaluate(addr => window.__node.connectPeer(addr), circuit);
  return { a, b };
}

// Debugging aid: with P2P_QA_DIAG_DIR set, keep each browser's console.
function dumpConsole(node, suffix) {
  if (!process.env.P2P_QA_DIAG_DIR) return;
  require('node:fs').writeFileSync(
    require('node:path').join(process.env.P2P_QA_DIAG_DIR, `${node.label}-${suffix}.console.log`),
    node.consoleLines.join('\n'),
  );
}

async function closeNodes(...nodes) {
  for (const node of nodes) dumpConsole(node, 'closed');
  for (const node of nodes) {
    await node.page.evaluate(() => window.__node.shutdown()).catch(() => {});
    await node.context.close().catch(() => {});
  }
}

async function diagnose(...nodes) {
  for (const node of nodes) dumpConsole(node, 'failed');
  for (const node of nodes) {
    const snap = await snapshot(node);
    const pulses = snap.pulses.filter(line => !line.includes('heartbeat'));
    log(`DIAG ${node.label}: upgrade=${JSON.stringify(snap.direct_upgrade)} pulses=${JSON.stringify(pulses.slice(-20))}`);
    const interesting = node.consoleLines.filter(line => /gossipsub|webrtc|signal|ICE|close|Closed|disconnect|relay|circuit|keep.?alive|idle/i.test(line));
    log(`DIAG ${node.label} console (${node.consoleLines.length} lines): ${JSON.stringify(interesting.slice(-40))}`);
    log(`DIAG ${node.label} events: ${JSON.stringify(await node.page.evaluate(() => window.__events.filter(e => !e.includes('local_binding')).slice(-20)))}`);
    log(`DIAG ${node.label} peers: ${JSON.stringify(await node.page.evaluate(async () => (await window.__node.getPeers()).map(p => [p.peer_id.slice(-6), p.connected])))}`);
  }
}

async function scenarioDirectSuccess(browser, baseUrl, info, harness) {
  const { a, b } = await browserPair(browser, baseUrl, info, { name: 'success' }, harness);
  const migrated = await waitFor('success: path migrated to direct WebRTC', async () => {
    const [sa, sb] = [await snapshot(a), await snapshot(b)];
    return sa.direct_upgrade.paths_migrated + sb.direct_upgrade.paths_migrated >= 1 &&
      sa.direct_upgrade.upgrades_succeeded + sb.direct_upgrade.upgrades_succeeded >= 2 &&
      [sa, sb];
  }, 60000).catch(async error => {
    await diagnose(a, b);
    log(`DIAG relay: ${JSON.stringify(await harness.stats())}`);
    throw error;
  });
  for (const node of [a, b]) {
    if (!sawPulse(node, 'WEBRTC_SIGNALING')) throw new Error(`success: ${node.label} had no WEBRTC_SIGNALING pulse`);
    if (!sawPulse(node, 'DIRECT_UPGRADE_SUCCESS')) throw new Error(`success: ${node.label} had no DIRECT_UPGRADE_SUCCESS pulse`);
    if (![...node.pulseLog].some(line => line.startsWith('ICE_CHECK') && line.includes('outcome=connected'))) {
      throw new Error(`success: ${node.label} had no ICE_CHECK outcome=connected pulse`);
    }
  }
  if (!sawPulse(a, 'PATH_MIGRATED_TO_DIRECT') && !sawPulse(b, 'PATH_MIGRATED_TO_DIRECT')) {
    throw new Error('success: no PATH_MIGRATED_TO_DIRECT pulse');
  }
  await waitFor('success: relayed circuit closed', async () =>
    (await harness.stats()).relayActiveCircuits === 0, 20000);
  await settle();
  const before = await harness.stats();
  await sendUntilDelivered(a, b.peerId, info.topic, MESSAGES);
  await sendUntilDelivered(b, a.peerId, info.topic, MESSAGES);
  await waitFor('success: messages delivered both ways', async () =>
    (await inboxSize(b)) >= MESSAGES && (await inboxSize(a)) >= MESSAGES).catch(async error => {
    log(`DIAG inbox a=${await inboxSize(a)} b=${await inboxSize(b)}`);
    await diagnose(a, b);
    throw error;
  });
  await settle();
  const after = await harness.stats();
  if (after.relayBytes !== before.relayBytes) {
    throw new Error(`success: relay still carried traffic after migration (${before.relayBytes} -> ${after.relayBytes} bytes)`);
  }
  log(`PASS: browser<->browser direct upgrade; relay transit frozen at ${after.relayBytes} bytes while ${MESSAGES * 2} messages flowed directly`);
  await closeNodes(a, b);
}

async function scenarioDirectFailure(browser, baseUrl, info, harness) {
  const { a, b } = await browserPair(browser, baseUrl, info, { name: 'failure', relayOnlyIce: true }, harness);
  const failed = await waitFor('failure: upgrade failed and fell back', async () => {
    const snaps = [await snapshot(a), await snapshot(b)];
    return [a, b].some(n => sawPulse(n, 'DIRECT_UPGRADE_FAILED')) &&
      [a, b].some(n => sawPulse(n, 'RELAY_FALLBACK')) && snaps;
  }, 90000);
  const iceFailure = /^ICE_CHECK .*outcome=(failed|timeout)/;
  if (![a, b].some(n => [...n.pulseLog].some(line => iceFailure.test(line)))) {
    throw new Error('failure: no ICE_CHECK failed/timeout pulse');
  }
  if (failed.some(s => s.direct_upgrade.paths_migrated > 0)) {
    throw new Error('failure: a direct path was reported despite forced ICE failure');
  }
  const before = await harness.stats();
  await sendUntilDelivered(a, b.peerId, info.topic, MESSAGES).catch(async error => {
    await diagnose(a, b);
    log(`DIAG relay: ${JSON.stringify(await harness.stats())}`);
    throw error;
  });
  await waitFor('failure: messages delivered over the relay', async () => (await inboxSize(b)) >= MESSAGES)
    .catch(async error => {
      log(`DIAG inbox b=${await inboxSize(b)}`);
      await diagnose(a, b);
      log(`DIAG relay: ${JSON.stringify(await harness.stats())}`);
      throw error;
    });
  const after = await relayBytesGrewPast(harness, before, 'failure');
  log(`PASS: forced direct failure keeps the relay path (${after.relayBytes - before.relayBytes} relayed bytes)`);
  await closeNodes(a, b);
}

async function scenarioBrowserToNativeDirect(browser, baseUrl, info, harness) {
  const node = await openNode(browser, baseUrl, 'native-direct', nodeConfig(info.relay.addr));
  const before = await harness.stats();
  await node.page.evaluate(addr => window.__node.connectPeer(addr), info.nativeDirect.addr);
  await waitFor('native-direct: DIRECT_DIAL recorded', async () => {
    await snapshot(node);
    return sawPulse(node, `DIRECT_DIAL peer=${info.nativeDirect.peerId}`);
  }, 30000);
  await sendUntilDelivered(node, info.nativeDirect.peerId, info.topic, MESSAGES);
  await waitFor('native-direct: native received', async () =>
    (await harness.stats()).nativeDirectReceived >= MESSAGES);
  await settle();
  const after = await harness.stats();
  if (after.relayBytes !== before.relayBytes) throw new Error('native-direct: traffic went through the relay');
  log('PASS: browser -> native over WebRTC-direct without relay transit');
  await closeNodes(node);
}

async function scenarioBrowserToNativeRelayFallback(browser, baseUrl, info, harness) {
  const node = await openNode(browser, baseUrl, 'native-relay', nodeConfig(info.relay.addr));
  await waitFor('native-relay: reserved', async () => (await snapshot(node)).relay_client_reservations > 0)
    .catch(async error => {
      await diagnose(node);
      log(`DIAG relay: ${JSON.stringify(await harness.stats())}`);
      throw error;
    });
  const before = await harness.stats();
  const circuit = `${info.relay.addr}/p2p-circuit/p2p/${info.nativeRelayOnly.peerId}`;
  await node.page.evaluate(addr => window.__node.connectPeer(addr), circuit);
  await waitFor('native-relay: fallback recorded', async () => {
    await snapshot(node);
    return sawPulse(node, 'RELAY_CONNECT') && sawPulse(node, 'RELAY_FALLBACK');
  }, 30000);
  await sendUntilDelivered(node, info.nativeRelayOnly.peerId, info.topic, MESSAGES);
  await waitFor('native-relay: native received via relay', async () =>
    (await harness.stats()).nativeRelayOnlyReceived >= MESSAGES);
  await relayBytesGrewPast(harness, before, 'native-relay');
  log('PASS: browser -> native relay-only peer works through the relay fallback');
  await closeNodes(node);
}

async function scenarioCapabilities(browser, baseUrl, info) {
  const node = await openNode(browser, baseUrl, 'capabilities', nodeConfig(info.relay.addr));
  const snap = await snapshot(node);
  for (const name of ['webrtc-browser', 'webtransport', 'webrtc-direct']) {
    if (!snap.active_transports.includes(name)) throw new Error(`capabilities: browser lacks ${name}`);
  }
  for (const name of ['tcp', 'quic', 'dcutr']) {
    if (snap.active_transports.includes(name)) throw new Error(`capabilities: browser advertises ${name}`);
  }
  log('PASS: browser advertises only the transports it actually runs');
  await closeNodes(node);
}

(async () => {
  const harness = startHarness();
  const info = await withTimeout('harness startup', harness.next(), 120000);
  const { server, baseUrl } = await servePackage(packageRoot());
  // Expose real host candidates instead of mDNS-obfuscated ones so two local
  // browser contexts can complete ICE without any STUN server.
  const browser = await chromium.launch({
    headless: true,
    args: ['--disable-features=WebRtcHideLocalIpsWithMdns'],
  });
  try {
    // P2P_QA_SCENARIOS=success,failure,... runs a subset (debugging aid).
    const only = (process.env.P2P_QA_SCENARIOS || '').split(',').filter(Boolean);
    const scenarios = [
      ['capabilities', () => scenarioCapabilities(browser, baseUrl, info)],
      ['success', () => scenarioDirectSuccess(browser, baseUrl, info, harness)],
      ['failure', () => scenarioDirectFailure(browser, baseUrl, info, harness)],
      ['native-direct', () => scenarioBrowserToNativeDirect(browser, baseUrl, info, harness)],
      ['native-relay', () => scenarioBrowserToNativeRelayFallback(browser, baseUrl, info, harness)],
    ];
    for (const [name, run] of scenarios) {
      if (only.length === 0 || only.includes(name)) await run();
    }
    log('PASS: browser relay-assisted direct upgrade QA');
  } finally {
    await browser.close().catch(() => {});
    await new Promise(resolve => server.close(resolve));
    harness.stop();
  }
})().catch(error => {
  process.stderr.write(`${error && error.stack ? error.stack : error}\n`);
  process.exitCode = 1;
});
