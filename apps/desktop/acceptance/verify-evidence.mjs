// Read only explicitly supplied synthetic reports; never modify campaign evidence.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { validateFullAcceptance } from './report.mjs';

const [browserPath, nativePath, backendProbePath, uiProbePath] = process.argv.slice(2);
assert.equal(process.argv.length, 6, 'supply browser, native, backend-probe and UI-probe result.json');
const checks = [];
for (const [filename, mode] of [[browserPath, 'browser-test-only-fetch-adapter'], [nativePath, 'native-webview2-plugin-http']]) {
  const report = JSON.parse(readFileSync(filename, 'utf8'));
  assert.equal(report.mode, mode);
  validateFullAcceptance(report);
  checks.push({ filename, check: 'real full campaign', result: 'PASS' });
  for (const [field, value] of [['client', 'a'], ['phase', 'offline-backlog']]) {
    const mutated = structuredClone(report);
    for (const h of mutated.history) if (h.client === 'b' && h.phase === 'backlog-reconnect') h[field] = value;
    assert.throws(() => validateFullAcceptance(mutated), /B reconnect/);
    checks.push({ filename, check: `in-memory wrong-${field} mutation rejected`, result: 'PASS' });
  }
}
for (const [filename, scope] of [[backendProbePath, 'backend-probe'], [uiProbePath, 'ui-probe']]) {
  const report = JSON.parse(readFileSync(filename, 'utf8'));
  assert.equal(report.scope, scope);
  assert.equal(report.fullCampaignComplete, false);
  assert.equal(report.result, 'PROBE_PASS');
  assert.throws(() => validateFullAcceptance(report));
  assert.throws(() => validateFullAcceptance({ ...report, result: 'PASS' }));
  checks.push({ filename, check: 'real probe rejected, including legacy PASS mutation', result: 'PASS' });
}
console.log(JSON.stringify({ checks }, null, 2));
