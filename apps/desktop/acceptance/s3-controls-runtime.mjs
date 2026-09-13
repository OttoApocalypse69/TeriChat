// Task-specific transport/deadline helpers; no application fixtures or secrets.
import assert from 'node:assert/strict';
import { renameSync, writeFileSync } from 'node:fs';

export async function bounded(operation, label, milliseconds = 15000) {
  let timer;
  try {
    return await Promise.race([operation(), new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(label)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

export async function completeForwardedResponse(response, expectedStatus, milliseconds = 15000) {
  assert.equal(response.status(), expectedStatus);
  // HTTP 204 has no body. Chromium/Playwright may never emit requestfinished
  // when the browser fetch consumer correctly does not read a 204 body.
  // For responses with bodies, require complete bytes within the deadline.
  if (expectedStatus !== 204) await bounded(() => response.body(), 'response body deadline', milliseconds);
}

export async function closeResources(closers, milliseconds = 10000) {
  const results = await Promise.allSettled(closers.map(close => bounded(close, 'cleanup deadline', milliseconds)));
  return results.every(result => result.status === 'fulfilled') ? 'PASS' : 'FAIL';
}

export function writeReport(file, report) {
  // A killed writer leaves the previous complete checkpoint readable.
  const temporary = `${file}.tmp`;
  writeFileSync(temporary, JSON.stringify(report, null, 2) + '\n');
  renameSync(temporary, file);
}
