// Diagnostic: inspect only the acceptance executable with an isolated profile.
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { chromium } from 'playwright-core';
const desktop = fileURLToPath(new URL('../', import.meta.url));
const id = new Date().toISOString().replace(/[:.]/g, '-');
const out = path.join(desktop, `evidence/client-acceptance/native-diagnostic-${id}`);
mkdirSync(out, { recursive: true });
const env = {};
for (const key of ['PATH','Path','SYSTEMROOT','SystemRoot','WINDIR','COMSPEC','PATHEXT','TEMP','TMP','USERPROFILE','LOCALAPPDATA','APPDATA']) if (process.env[key] !== undefined) env[key] = process.env[key];
const profile = path.join(desktop, `.acceptance/native-diagnostic-${id}`);
const listener = createServer(); await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port; await new Promise(r => listener.close(r));
const app = spawn(path.join(desktop, '.acceptance/native-target/debug/terichat-desktop.exe'), [], { cwd: path.join(desktop, '.acceptance'), env: { ...env, WEBVIEW2_USER_DATA_FOLDER: profile, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --remote-debugging-address=127.0.0.1` }, stdio: 'ignore' });
let browser;
try {
  const deadline = Date.now() + 10000;
  while (true) {
    try { if ((await fetch(`http://127.0.0.1:${port}/json/version`)).ok) break; } catch {}
    if (Date.now() > deadline) throw new Error('CDP unavailable');
    await new Promise(r => setTimeout(r, 100));
  }
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const report = [];
  for (const context of browser.contexts()) for (const page of context.pages()) {
    const errors = [];
    page.on('pageerror', e => errors.push(e.message));
    await page.reload(); await page.waitForLoadState('load');
    report.push({ url: page.url(), title: await page.title(), body: await page.locator('body').innerText(), html: await page.content(), errors });
    await page.screenshot({ path: path.join(out, `page-${report.length}.png`) });
  }
  writeFileSync(path.join(out, 'pages.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report.map(({ html, ...rest }) => rest), null, 2));
  console.log(`Evidence: ${out}`);
} finally {
  await browser?.close().catch(() => {});
  if (app.exitCode === null) execFileSync('taskkill.exe', ['/PID', String(app.pid), '/T', '/F'], { env, stdio: 'pipe' });
  await new Promise(r => app.exitCode !== null || app.signalCode !== null ? r() : app.once('exit', r));
  rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 });
}
