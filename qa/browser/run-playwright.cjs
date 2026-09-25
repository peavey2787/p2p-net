'use strict';

const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { chromium, firefox } = require('playwright');

const STEP_TIMEOUT_MS = Number(process.env.P2P_PLAYWRIGHT_STEP_TIMEOUT_MS || 45000);

const pkgDir = process.env.P2P_WASM_PKG_DIR;
if (!pkgDir) {
  throw new Error('P2P_WASM_PKG_DIR is required');
}
const root = path.resolve(pkgDir);
const jsEntry = path.join(root, 'p2p_net.js');
const wasmEntry = path.join(root, 'p2p_net_bg.wasm');
for (const required of [jsEntry, wasmEntry]) {
  if (!fs.existsSync(required)) {
    throw new Error(`missing wasm-pack browser artifact: ${required}`);
  }
}

const indexHtml = `<!doctype html>
<meta charset="utf-8">
<title>p2p-net Playwright WASM QA</title>
<script type="module">
  try {
    const mod = await import('./p2p_net.js');
    await mod.default();
    window.__p2pNet = mod;
    window.__p2pNetReady = true;
  } catch (error) {
    window.__p2pNetError = String(error && error.stack ? error.stack : error);
  }
</script>`;

function contentType(filePath) {
  switch (path.extname(filePath)) {
    case '.js': return 'text/javascript; charset=utf-8';
    case '.wasm': return 'application/wasm';
    case '.json': return 'application/json; charset=utf-8';
    case '.html': return 'text/html; charset=utf-8';
    default: return 'application/octet-stream';
  }
}

function safePath(urlPath) {
  const decoded = decodeURIComponent(urlPath.split('?')[0]);
  const relative = decoded.replace(/^\/+/, '');
  const resolved = path.resolve(root, relative);
  if (resolved !== root && !resolved.startsWith(root + path.sep)) {
    return null;
  }
  return resolved;
}

const server = http.createServer((req, res) => {
  if (!req.url || req.url === '/' || req.url.startsWith('/index.html')) {
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' });
    res.end(indexHtml);
    return;
  }

  const filePath = safePath(req.url);
  if (!filePath || !fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
    res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
    res.end('not found');
    return;
  }
  res.writeHead(200, { 'Content-Type': contentType(filePath), 'Cache-Control': 'no-store' });
  fs.createReadStream(filePath).pipe(res);
});

function listen() {
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => resolve(server.address()));
  });
}

function logStep(name, step) {
  process.stdout.write(`RUN: Playwright ${name}: ${step}\n`);
}

async function withTimeout(label, promise, timeoutMs = STEP_TIMEOUT_MS) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`${label} timed out after ${timeoutMs} ms`)),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

async function waitForWasm(page, name) {
  await withTimeout(
    `${name}: WASM initialization`,
    page.waitForFunction(() => window.__p2pNetReady === true || Boolean(window.__p2pNetError)),
  );
  const error = await page.evaluate(() => window.__p2pNetError || null);
  if (error) throw new Error(`WASM initialization failed: ${error}`);
}

async function runBrowser(name, browserType, baseUrl) {
  logStep(name, 'launch browser');
  const browser = await withTimeout(`${name}: browser launch`, browserType.launch({ headless: true }));
  const context = await browser.newContext();
  const page = await context.newPage();
  page.setDefaultTimeout(STEP_TIMEOUT_MS);
  page.setDefaultNavigationTimeout(STEP_TIMEOUT_MS);
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(String(error && error.stack ? error.stack : error)));
  page.on('console', message => {
    if (message.type() === 'error') pageErrors.push(`console.error: ${message.text()}`);
  });

  try {
    logStep(name, 'load WASM page');
    await page.goto(baseUrl, { waitUntil: 'load' });
    await waitForWasm(page, name);

    logStep(name, 'verify browser exports');
    const exportCheck = await page.evaluate(() => ({
      wasmNode: typeof window.__p2pNet.WasmNode,
      storageWrite: typeof window.__p2pNet.qaStorageWrite,
      storageRead: typeof window.__p2pNet.qaStorageRead,
      profileLifecycle: typeof window.__p2pNet.qaProfileLifecycle,
    }));
    for (const [exportName, kind] of Object.entries(exportCheck)) {
      if (kind !== 'function') throw new Error(`${name}: missing browser export ${exportName}`);
    }

    const storageNamespace = `qa-storage-${name}-${Date.now()}-${Math.random()}`;
    logStep(name, 'write IndexedDB journal');
    await withTimeout(
      `${name}: qaStorageWrite`,
      page.evaluate(namespace => window.__p2pNet.qaStorageWrite(namespace), storageNamespace),
    );

    // This is a real document reload, not merely a second Rust object in the
    // same JS realm. It verifies IndexedDB durability across browser lifecycle.
    logStep(name, 'reload page');
    await page.reload({ waitUntil: 'load' });
    await waitForWasm(page, name);

    logStep(name, 'read IndexedDB journal after reload');
    await withTimeout(
      `${name}: qaStorageRead`,
      page.evaluate(namespace => window.__p2pNet.qaStorageRead(namespace), storageNamespace),
    );

    const nodeNamespace = `qa-node-${name}-${Date.now()}-${Math.random()}`;
    logStep(name, 'verify profile lock + PeerId persistence lifecycle');
    const peerId = await withTimeout(
      `${name}: qaProfileLifecycle`,
      page.evaluate(async namespace => {
        try {
          return await window.__p2pNet.qaProfileLifecycle(namespace);
        } catch (error) {
          const kind = error && error.kind ? `${error.kind}: ` : '';
          const message = error && error.message ? error.message : String(error);
          throw new Error(`${kind}${message}`);
        }
      }, nodeNamespace),
      STEP_TIMEOUT_MS * 2,
    );
    if (typeof peerId !== 'string' || peerId.length === 0) {
      throw new Error(`${name}: browser node returned an empty PeerId`);
    }

    if (pageErrors.length) {
      throw new Error(`${name}: browser errors observed:\n${pageErrors.join('\n')}`);
    }
    process.stdout.write(`PASS: Playwright ${name} WASM browser parity (PeerId ${peerId})\n`);
  } finally {
    await context.close().catch(() => {});
    await browser.close().catch(() => {});
  }
}

(async () => {
  const address = await listen();
  const baseUrl = `http://127.0.0.1:${address.port}/`;
  process.stdout.write(`Playwright WASM QA server: ${baseUrl}\n`);
  try {
    await runBrowser('chromium', chromium, baseUrl);
    await runBrowser('firefox', firefox, baseUrl);
  } finally {
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => {
  process.stderr.write(`${error && error.stack ? error.stack : error}\n`);
  process.exitCode = 1;
});
