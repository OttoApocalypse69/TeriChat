import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export function validateFullAcceptance(report) {
  assert.equal(report.result, 'PASS');
  assert.equal(report.scope, 'full-campaign');
  assert.equal(report.fullCampaignComplete, true);
  assert.ok(Array.isArray(report.arguments), 'explicit arguments required');
  assert.deepEqual(report.arguments, report.mode === 'native-webview2-plugin-http' ? ['--native'] : []);
  assert.ok(['native-webview2-plugin-http', 'browser-test-only-fetch-adapter'].includes(report.mode));
  for (const name of [
    'isolated backend startup and readiness',
    'two clients login; A sends bro; B receives live; plaintext warning visible',
    '125-message offline backlog restored in sequence with no gaps or duplicates',
    'second reconnect preserves exact message sequence',
    'quiet conversation recovers failed history on reconnect without a new message',
    'account switch clears other account private conversation and restores own history',
    'full renderer reload/login restores all history without duplicates',
  ]) assert.ok(report.steps.some(step => step.name === name), `missing scenario: ${name}`);
  assert.equal(report.steps.find(step => step.name.startsWith('125-message')).total, 126);
  assert.ok(report.backlogConversation, 'backlog conversation required');
  validateHistory(report.history, report.backlogConversation);
  assert.equal(report.containerRemoved, true);
  assert.equal(report.volumesRemoved, true);
  assert.deepEqual(report.cleanupErrors, []);
  if (report.mode === 'native-webview2-plugin-http') {
    assert.match(report.nativeExecutableSha256, /^[a-f0-9]{64}$/);
    for (const client of ['a', 'b']) assert.ok(report.steps.some(step => step.name === `native ${client} window` && step.MainWindowHandle !== 0 && Number.isInteger(step.MainWindowHandle)), 'native window required');
  }
}

// Machine gate: fail closed on old reports and probes. No runtime resources used.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  assert.ok(process.argv.length > 2, 'supply result.json paths');
  for (const filename of process.argv.slice(2)) {
    validateFullAcceptance(JSON.parse(readFileSync(filename, 'utf8')));
    console.log(`FULL ACCEPTANCE PASS: ${filename}`);
  }
}


export function validateHistory(history, conversation) {
  const pages = history.filter(h => h.client === 'b' && h.phase === 'backlog-reconnect' && h.conversation === conversation && h.status === 200);
  assert.ok(pages.length >= 2, 'B reconnect needs two successful history pages');
  assert.deepEqual(pages.slice(0, 2).map(({ since, limit, size }) => ({ since, limit, size })), [
    { since: 1, limit: 100, size: 100 },
    { since: 101, limit: 100, size: 25 },
  ], 'B reconnect exact page progression and response sizes');
}
