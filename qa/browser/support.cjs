'use strict';

// Shared helpers for the Playwright browser QA runners: serve a wasm-pack
// `--target web` package on loopback and wait for it to initialize.

const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');

const STEP_TIMEOUT_MS = Number(process.env.P2P_PLAYWRIGHT_STEP_TIMEOUT_MS || 45000);

function packageRoot() {
  const pkgDir = process.env.P2P_WASM_PKG_DIR;
  if (!pkgDir) throw new Error('P2P_WASM_PKG_DIR is required');
  const root = path.resolve(pkgDir);
  for (const required of ['p2p_net.js', 'p2p_net_bg.wasm']) {
    if (!fs.existsSync(path.join(root, required))) {
      throw new Error(`missing wasm-pack browser artifact: ${path.join(root, required)}`);
    }
  }
  return root;
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

function safePath(root, urlPath) {
  const decoded = decodeURIComponent(urlPath.split('?')[0]);
  const relative = decoded.replace(/^\/+/, '');
  const resolved = path.resolve(root, relative);
  if (resolved !== root && !resolved.startsWith(root + path.sep)) {
    return null;
  }
  return resolved;
}

/** Serve the package on 127.0.0.1; resolves to `{ server, baseUrl }`. */
function servePackage(root) {
  const server = http.createServer((req, res) => {
    if (!req.url || req.url === '/' || req.url.startsWith('/index.html')) {
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' });
      res.end(indexHtml);
      return;
    }
    const filePath = safePath(root, req.url);
    if (!filePath || !fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
      res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      res.end('not found');
      return;
    }
    res.writeHead(200, { 'Content-Type': contentType(filePath), 'Cache-Control': 'no-store' });
    fs.createReadStream(filePath).pipe(res);
  });
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      resolve({ server, baseUrl: `http://127.0.0.1:${server.address().port}/` });
    });
  });
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

module.exports = {
  STEP_TIMEOUT_MS,
  packageRoot,
  servePackage,
  waitForWasm,
  withTimeout,
};
