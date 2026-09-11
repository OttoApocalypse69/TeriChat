// Independent out-of-harness check for exe-restart campaigns.
// Reads only the supplied synthetic result.json; never modifies evidence.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const [restartPath] = process.argv.slice(2);
assert.equal(process.argv.length, 3, 'supply restart result.json');

const checks = [];

function validateRestart(report) {
  assert.equal(report.scope, 'exe-restart-campaign');
  assert.equal(report.mode, 'native-webview2-plugin-http-exe-restart');
  assert.equal(report.result, 'PASS');
  assert.equal(report.fullRestartComplete, true);
  const gen1 = report.generations.gen1.pids;
  const gen2 = report.generations.gen2.pids;
  assert.equal(gen1.length, 2);
  assert.equal(gen2.length, 2);
  for (const pid of gen2) assert.ok(!gen1.includes(pid), 'gen2 pids differ from gen1');
  for (const [name, dump, total] of [['preHistory', report.preHistory, report.preRestartTotal], ['postHistory', report.postHistory, report.postRestartTotal]]) {
    assert.equal(dump.length, total, `${name} length matches stage total`);
    assert.deepEqual(dump.map((m) => m.seq), dump.map((_, i) => i + 1), `${name} seqs contiguous from 1`);
    assert.equal(new Set(dump.map((m) => m.client_msg_id)).size, dump.length, `${name} no duplicate ids`);
    for (const m of dump) assert.match(m.contentHash, /^[a-f0-9]{16}$/, `${name} content hash present`);
  }
  assert.deepEqual(report.postHistory.map((m) => m.client_msg_id), report.preHistory.map((m) => m.client_msg_id), 'restart preserved exact message set');
  assert.equal(report.finalHistory.length, report.preHistory.length + 1, 'one post-restart send appended');
  assert.deepEqual(report.finalHistory.slice(0, -1).map((m) => m.client_msg_id), report.preHistory.map((m) => m.client_msg_id), 'prefix untouched by post-restart send');
  assert.equal(report.containerRemoved, true);
  assert.equal(report.volumesRemoved, true);
  assert.deepEqual(report.cleanupErrors, []);
  assert.match(report.exeSha256, /^[a-f0-9]{64}$/);
}

const report = JSON.parse(readFileSync(restartPath, 'utf8'));
validateRestart(report);
checks.push({ filename: restartPath, check: 'real restart campaign', result: 'PASS' });

// In-memory mutations must fail validation: dropped message, duplicated id,
// gen2 pid colliding with gen1, forged completion.
const dropped = structuredClone(report);
dropped.postHistory = dropped.postHistory.slice(1);
assert.throws(() => validateRestart(dropped), /length matches/);
checks.push({ filename: restartPath, check: 'dropped-message mutation rejected', result: 'PASS' });

const duped = structuredClone(report);
duped.preHistory[1] = structuredClone(duped.preHistory[0]);
assert.throws(() => validateRestart(duped));
checks.push({ filename: restartPath, check: 'duplicate-id mutation rejected', result: 'PASS' });

const pidForge = structuredClone(report);
pidForge.generations.gen2.pids = [...report.generations.gen1.pids];
assert.throws(() => validateRestart(pidForge), /differ/);
checks.push({ filename: restartPath, check: 'pid-reuse mutation rejected', result: 'PASS' });

const probeForge = structuredClone(report);
delete probeForge.fullRestartComplete;
assert.throws(() => validateRestart(probeForge));
checks.push({ filename: restartPath, check: 'incomplete-campaign mutation rejected', result: 'PASS' });

console.log(JSON.stringify({ checks }, null, 2));
