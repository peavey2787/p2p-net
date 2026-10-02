'use strict';

const { chromium, firefox } = require('playwright');
const {
  STEP_TIMEOUT_MS,
  packageRoot,
  servePackage,
  waitForWasm,
  withTimeout,
} = require('./support.cjs');

function logStep(name, step) {
  process.stdout.write(`RUN: Playwright ${name}: ${step}\n`);
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
  const { server, baseUrl } = await servePackage(packageRoot());
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
