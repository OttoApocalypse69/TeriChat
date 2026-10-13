const base = 'http://127.0.0.1:3001';
async function api(method, route, body, token) {
  const r = await fetch(base + route, { method, headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) }, body: body && JSON.stringify(body) });
  const t = await r.text(); if (!r.ok) throw new Error(`${route} ${r.status} ${t}`); return t ? JSON.parse(t) : null;
}
const [cmd, ...args] = process.argv.slice(2);
const password = 'synthetic-smoke-password-1';
if (cmd === 'seed') {
  for (const [h, d] of [['notify_a', 'Smoke A'], ['notify_b', 'Smoke Bee']]) await api('POST', '/v1/auth/register', { handle: h, email: `${h}@example.invalid`, display_name: d, password });
  const b = await api('POST', '/v1/auth/login', { handle: 'notify_b', password });
  const dm = await api('POST', '/v1/conversations/dm', { peer_handle: 'notify_a' }, b.token);
  console.log(JSON.stringify({ dm: dm.id }));
} else if (cmd === 'send') {
  const b = await api('POST', '/v1/auth/login', { handle: 'notify_b', password });
  const dm = (await api('GET', '/v1/conversations', undefined, b.token)).find(c => c.peer_handle === 'notify_a');
  const m = await api('POST', '/v1/messages', { conversation_id: dm.id, client_msg_id: crypto.randomUUID(), ciphertext_b64: Buffer.from(args[0]).toString('base64') }, b.token);
  console.log(JSON.stringify({ seq: m.seq }));
}
