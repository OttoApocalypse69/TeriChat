import assert from 'node:assert/strict';
import test from 'node:test';

import { validateHistory, validateFullAcceptance } from './report.mjs';
const page = (since, size, extra = {}) => ({ client: 'b', phase: 'backlog-reconnect', conversation: 'dm', since, limit: 100, status: 200, size, ...extra });
const valid = () => [page(1, 100), page(101, 25)];
test('B reconnect successful pages use exact cursors and response sizes', () => {
  assert.doesNotThrow(() => validateHistory(valid(), 'dm'));
});
for (const [name, history] of [
  ['wrong client', valid().map(p => ({ ...p, client: 'a' }))],
  ['wrong phase', valid().map(p => ({ ...p, phase: 'offline-backlog' }))],
  ['unsuccessful response', valid().map(p => ({ ...p, status: 503 }))],
  ['wrong first cursor', [page(0, 100), page(101, 25)]],
  ['wrong page size', [page(1, 99), page(101, 25)]],
  ['reversed page progression', valid().reverse()],
]) test(`reject ${name}`, () => assert.throws(() => validateHistory(history, 'dm')));

const campaign = () => ({ result: 'PASS', mode: 'browser-test-only-fetch-adapter', scope: 'full-campaign', arguments: [], fullCampaignComplete: true,
  steps: [
    'isolated backend startup and readiness',
    'two clients login; A sends bro; B receives live; plaintext warning visible',
    '125-message offline backlog restored in sequence with no gaps or duplicates',
    'second reconnect preserves exact message sequence',
    'quiet conversation recovers failed history on reconnect without a new message',
    'account switch clears other account private conversation and restores own history',
    'full renderer reload/login restores all history without duplicates',
  ].map(name => ({ name, ...(name.startsWith('125-') ? { total: 126 } : {}) })),
  backlogConversation: 'dm', history: valid(), containerRemoved: true, volumesRemoved: true, cleanupErrors: [],
});
test('complete campaign accepted', () => assert.doesNotThrow(() => validateFullAcceptance(campaign())));
for (const [name, changes] of [
  ['native backend probe despite legacy PASS/mode', { mode: 'native-webview2-plugin-http', arguments: ['--native', '--probe-only'], scope: 'backend-probe', fullCampaignComplete: false }],
  ['UI probe despite legacy PASS', { arguments: ['--ui-probe'], scope: 'ui-probe', fullCampaignComplete: false }],
  ['probe arguments with forged completion', { arguments: ['--probe-only'] }],
  ['missing completion', { fullCampaignComplete: undefined }],
  ['missing steps', { steps: [] }],
  ['wrong-client full report', { history: valid().map(p => ({ ...p, client: 'a' })) }],
  ['wrong-phase full report', { history: valid().map(p => ({ ...p, phase: 'live' })) }],
  ['cleanup failure', { cleanupErrors: ['resource remained'] }],
]) test(`full acceptance rejects ${name}`, () => assert.throws(() => validateFullAcceptance({ ...campaign(), ...changes })));
