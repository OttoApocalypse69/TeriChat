// Real loopback HTTP transport regression. No database/application acceptance.
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, rmdirSync } from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { chromium } from 'playwright-core';
import { bounded, closeResources, completeForwardedResponse, writeReport } from './s3-controls-runtime.mjs';

test('unread real HTTP 204 completes without waiting for requestfinished', { timeout: 20000 }, async () => {
  const server = http.createServer((req, res) => {
    if (req.url === '/no-content') { res.writeHead(204); res.end(); return; }
    if (req.url === '/body') { res.end('synthetic body'); return; }
    if (req.url === '/stalled-body') { res.writeHead(200, { 'content-length': '100' }); res.flushHeaders(); return; }
    res.end('Synthetic transport regression');
  });
  let browser;
  try {
    await bounded(() => new Promise(resolve => server.listen(0, '127.0.0.1', resolve)), 'server startup');
    // Local diagnosis may use an already-installed executable; hosted CI leaves
    // this unset and uses the exact browser installed by pinned playwright-core.
    browser = await chromium.launch({ headless: true,
      ...(process.env.S3_BROWSER_EXECUTABLE ? { executablePath: process.env.S3_BROWSER_EXECUTABLE } : {}) });
    const page = await browser.newPage();
    page.setDefaultTimeout(5000);
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    const received = page.waitForResponse(r => new URL(r.url()).pathname === '/no-content');
    await page.evaluate(() => {
      window.transportDone = false;
      // Same relevant behavior as ApiClient.req: 204 returns without body read.
      void fetch('/no-content', { method: 'DELETE' }).then(response => {
        if (response.status !== 204) throw new Error('unexpected status');
        window.transportDone = true;
      });
    });
    const response = await received;
    await bounded(() => completeForwardedResponse(response, 204), '204 completion stalled', 2000);
    await page.waitForFunction(() => window.transportDone === true);
    await assert.rejects(completeForwardedResponse(response, 200), { name: 'AssertionError' });

    const bodyReceived = page.waitForResponse(r => new URL(r.url()).pathname === '/body');
    await page.evaluate(() => { void fetch('/body').then(response => response.text()); });
    await completeForwardedResponse(await bodyReceived, 200);
    const stalledReceived = page.waitForResponse(r => new URL(r.url()).pathname === '/stalled-body');
    await page.evaluate(() => { void fetch('/stalled-body').then(response => response.text()).catch(() => {}); });
    await assert.rejects(completeForwardedResponse(await stalledReceived, 200, 100), /response body deadline/);
  } finally {
    server.closeAllConnections();
    assert.equal(await closeResources([
      () => browser?.close(),
      () => new Promise(resolve => server.close(resolve)),
    ], 5000), 'PASS');
  }
});

test('stalled operations and cleanup fail within deadlines and leave readable failure progress', async () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), 's3-runtime-'));
  const file = path.join(directory, 'result.json');
  try {
    const report = { result: 'FAIL', phase: 'synthetic transport regression', operation: 'started' };
    writeReport(file, report);
    await assert.rejects(bounded(() => new Promise(() => {}), 'operation deadline', 20), /operation deadline/);
    report.cleanup = await closeResources([() => new Promise(() => {}), () => Promise.resolve()], 20);
    assert.equal(report.cleanup, 'FAIL');
    writeReport(file, report);
    assert.deepEqual(JSON.parse(readFileSync(file, 'utf8')), report);
    assert.equal(await closeResources([() => Promise.reject(new Error('synthetic close failure'))], 20), 'FAIL');
  } finally {
    rmSync(file, { force: true });
    rmSync(`${file}.tmp`, { force: true });
    rmdirSync(directory);
  }
});
