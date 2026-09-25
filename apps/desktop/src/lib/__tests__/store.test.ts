import { describe, expect, it } from 'vitest';
import {
  avatarInitial,
  ChatStore,
  conversationLabel,
  conversationSublabel,
  dayLabel,
  formatClockTime,
  formatListTime,
  gatewayEventInfo,
  lastActivityAt,
  mergeMessages,
  parseMemberHandles,
  senderLabel,
  sortConversations,
  truncatePreview,
  unreadBadge,
  unreadCount,
  upsertConversation,
  type ChatConversation,
  type ChatMessage,
} from '../store';

function msg(over: Partial<ChatMessage> & { id: string }): ChatMessage {
  return {
    conversation_id: 'conv-1',
    sender_id: 'alice',
    seq: 1,
    ciphertext_b64: 'YnJv',
    nonce_b64: null,
    client_msg_id: over.id,
    sent_at: '2026-01-01T00:00:00Z',
    ...over,
  };
}

describe('send/receive merge without duplicates', () => {
  it('merges history and live echoes by id without duplicates', () => {
    const history = [msg({ id: 'm1', seq: 1 }), msg({ id: 'm2', seq: 2 })];
    const liveEcho = [msg({ id: 'm2', seq: 2 })]; // gateway-triggered refetch
    const merged = mergeMessages(history, liveEcho);
    expect(merged.map((m) => m.id)).toEqual(['m1', 'm2']);
  });

  it('dedupes idempotent send retries by client_msg_id', () => {
    const sent = msg({ id: 'm1', seq: 1, client_msg_id: 'ck-1' });
    const retry = msg({ id: 'm1', seq: 1, client_msg_id: 'ck-1' });
    // Same client key, server-echoed row (same id here; key covers renames).
    const renamed = msg({ id: 'm1-dup', seq: 1, client_msg_id: 'ck-1' });
    expect(mergeMessages([sent], [retry]).map((m) => m.id)).toEqual(['m1']);
    expect(mergeMessages([sent], [renamed]).map((m) => m.id)).toEqual(['m1']);
  });

  it('keeps messages sorted by seq', () => {
    const merged = mergeMessages(
      [msg({ id: 'm3', seq: 3 })],
      [msg({ id: 'm1', seq: 1 }), msg({ id: 'm2', seq: 2 })],
    );
    expect(merged.map((m) => m.seq)).toEqual([1, 2, 3]);
  });

  it('upserts conversations without duplicating the list', () => {
    const one = upsertConversation([], {
      id: 'c1',
      kind: 'dm',
      members: ['a', 'b'],
    });
    const two = upsertConversation(one, {
      id: 'c1',
      kind: 'dm',
      members: ['a', 'b'],
    });
    expect(two).toHaveLength(1);
  });

  it('store mergeHistory is idempotent across repeated history pulls', () => {
    const store = new ChatStore();
    const page = [msg({ id: 'm1', seq: 1 }), msg({ id: 'm2', seq: 2 })];
    store.mergeHistory('conv-1', page);
    store.mergeHistory('conv-1', [...page]);
    expect(store.messages.get('conv-1')?.map((m) => m.id)).toEqual([
      'm1',
      'm2',
    ]);
  });
});

describe('conversation list display helpers', () => {
  const dm = (over: Partial<ChatConversation> = {}): ChatConversation => ({
    id: 'c-dm',
    kind: 'dm',
    members: ['me', 'peer'],
    peer_handle: 'bob',
    peer_display_name: 'Bob',
    last_seq: 4,
    last_sent_at: '2026-03-04T10:00:00Z',
    ...over,
  });

  it('labels DMs with the peer name, never a UUID', () => {
    expect(conversationLabel(dm())).toBe('Bob');
    expect(conversationLabel(dm({ peer_display_name: null }))).toBe('@bob');
    expect(
      conversationLabel(dm({ peer_display_name: null, peer_handle: null })),
    ).toBe('Direct message');
    expect(conversationSublabel(dm())).toBe('@bob');
    expect(conversationSublabel(dm({ peer_display_name: null }))).toBeNull();
  });

  it('derives avatar initials from the peer name first', () => {
    expect(avatarInitial(dm())).toBe('B');
    expect(avatarInitial(dm({ peer_display_name: null }))).toBe('B');
    expect(
      avatarInitial({ id: 'g', kind: 'group', members: ['a', 'b'] }),
    ).toBe('G');
  });

  it('tags senders as you/peer, not raw ids', () => {
    expect(senderLabel('me', 'me', dm())).toBe('you');
    expect(senderLabel('me', 'peer', dm())).toBe('Bob');
    expect(senderLabel('me', 'peer', dm({ peer_display_name: null }))).toBe(
      '@bob',
    );
    expect(senderLabel('me', 'abcdef123456', null)).toBe('abcdef12');
  });

  it('upsert keeps peer/last-message fields across list reloads', () => {
    const afterList = upsertConversation([], dm());
    expect(afterList[0].peer_display_name).toBe('Bob');
    expect(afterList[0].last_seq).toBe(4);
    // A later create-DM echo without summary fields must not wipe them.
    const afterEcho = upsertConversation(afterList, {
      id: 'c-dm',
      kind: 'dm',
      members: ['me', 'peer'],
    });
    expect(afterEcho[0].peer_display_name).toBe('Bob');
    expect(afterEcho).toHaveLength(1);
  });

  it('buckets day dividers into Today/Yesterday/date', () => {
    expect(dayLabel('2026-03-04T23:00:00Z', '2026-03-04T12:00:00Z')).toBe(
      'Today',
    );
    expect(dayLabel('2026-03-03T23:00:00Z', '2026-03-04T12:00:00Z')).toBe(
      'Yesterday',
    );
    expect(dayLabel('2026-03-01T00:00:00Z', '2026-03-04T12:00:00Z')).toBe(
      '2026-03-01',
    );
    expect(dayLabel('not-a-date', '2026-03-04T12:00:00Z')).toBe('');
  });

  it('formats clock times and list timestamps without throwing', () => {
    expect(formatClockTime(null)).toBe('');
    expect(formatClockTime('bogus')).toBe('');
    expect(formatClockTime('2026-03-04T10:05:00Z')).toMatch(/^\d{2}:\d{2}$/);
    expect(
      formatListTime('2026-03-04T10:05:00Z', '2026-03-04T12:00:00Z'),
    ).toMatch(/^\d{2}:\d{2}$/);
    expect(
      formatListTime('2026-03-03T10:05:00Z', '2026-03-04T12:00:00Z'),
    ).toBe('Yesterday');
    expect(formatListTime(null)).toBe('');
  });

  it('truncates previews to a single capped line', () => {
    expect(truncatePreview('  hello   world  ')).toBe('hello world');
    expect(truncatePreview('x'.repeat(100))).toHaveLength(60);
    expect(truncatePreview('x'.repeat(100)).endsWith('…')).toBe(true);
  });

  it('sorts newest activity first with message fallback', () => {
    const quiet = dm({ id: 'c-quiet', last_sent_at: '2026-01-01T00:00:00Z' });
    const loud = dm({ id: 'c-loud', last_sent_at: null, last_seq: null });
    const msgs = new Map([
      ['c-loud', [msg({ id: 'm9', conversation_id: 'c-loud', seq: 9 })]],
    ]);
    expect(lastActivityAt(quiet)).toBe('2026-01-01T00:00:00Z');
    expect(lastActivityAt(loud, [])).toBeNull();
    expect(sortConversations([quiet, loud], msgs).map((c) => c.id)).toEqual([
      'c-loud',
      'c-quiet',
    ]);
  });
});

describe('gateway event routing', () => {
  it('extracts conversation routing from message.created events', () => {
    const info = gatewayEventInfo({
      id: 'e1',
      topic: 'message.created',
      payload: {
        conversation_id: 'conv-9',
        data: { message_id: 'm9', seq: 4 },
      },
    });
    expect(info).toMatchObject({
      conversationId: 'conv-9',
      messageId: 'm9',
      seq: 4,
    });
    expect(
      gatewayEventInfo({ id: 'e2', topic: 'other', payload: {} }),
    ).toBeNull();
  });
});

describe('group rosters', () => {
  const profile = (user_id: string, handle: string, display_name = '') => ({ user_id, handle, display_name });
  const group = (profiles = [profile('me', 'teri', 'Teri'), profile('u1', 'ana', 'Ana'), profile('u2', 'bo')]): ChatConversation => ({
    id: 'g', kind: 'group', members: profiles.map(p => p.user_id), member_profiles: profiles,
  });

  it('names a group by its other members, never the caller or raw ids', () => {
    expect(conversationLabel(group(), 'me')).toBe('Ana, @bo');
    expect(conversationSublabel(group())).toBe('3 members');
    const big = group(['me', 'a', 'b', 'c', 'd', 'e'].map(id => profile(id, `h${id}`, `N${id}`)));
    expect(conversationLabel(big, 'me')).toBe('Na, Nb, Nc +2');
  });

  it('falls back to a member count when the server sent no roster', () => {
    const legacy: ChatConversation = { id: 'g', kind: 'group', members: ['me', 'u1', 'u2'] };
    expect(conversationLabel(legacy, 'me')).toBe('Group · 3 members');
    expect(conversationSublabel(legacy)).toBeNull();
  });

  it('labels group senders from the roster', () => {
    expect(senderLabel('me', 'u1', group())).toBe('Ana');
    expect(senderLabel('me', 'u2', group())).toBe('@bo');
    expect(senderLabel('me', 'me', group())).toBe('you');
    expect(senderLabel('me', 'stranger-id-1234', group())).toBe('stranger');
  });

  it('parses member handles: separators, @ prefixes, duplicates and self', () => {
    // Commas and new lines separate; spaces can be part of a handle.
    expect(parseMemberHandles(' @ana, mary jane,,@ana\n@teri ', 'teri')).toEqual(['ana', 'mary jane']);
    expect(parseMemberHandles(' , @ ', 'teri')).toEqual([]);
    // Only the display "@" is stripped: a handle may itself start with "@".
    expect(parseMemberHandles('@@odd')).toEqual(['@odd']);
    // Handles are case-insensitive server-side: one person, and never yourself.
    expect(parseMemberHandles('Ana, ana, ANA')).toEqual(['ana']);
    expect(parseMemberHandles('@TERI, bo', 'teri')).toEqual(['bo']);
    expect(parseMemberHandles('@teri', 'Teri')).toEqual([]);
  });
});

describe('private unread state', () => {
  const msg = (seq: number, sender = 'peer'): ChatMessage => ({
    id: `m${seq}`, conversation_id: 'c', sender_id: sender, seq, ciphertext_b64: '', client_msg_id: `c${seq}`, sent_at: '',
  });
  const conv = (over: Partial<ChatConversation> = {}): ChatConversation => ({
    id: 'c', kind: 'dm', members: ['me', 'peer'], last_seq: 5, last_read_seq: 2, ...over,
  });
  const five = [1, 2, 3, 4, 5].map(seq => msg(seq, seq === 4 ? 'me' : 'peer'));

  it('counts only other people’s messages after the marker', () => {
    expect(unreadCount(conv(), five, 'me')).toBe(2);
    expect(unreadCount(conv({ last_read_seq: 5 }), five, 'me')).toBe(0);
  });

  it('counts server-reported positions that history has not loaded yet', () => {
    expect(unreadCount(conv({ last_seq: 9 }), [msg(3)], 'me')).toBe(7);
    expect(unreadCount(conv(), undefined, 'me')).toBe(3);
  });

  it('keeps a gap below your own loaded reply unread', () => {
    // Peer's seq 3 failed to load; the caller's reply (seq 4) did.
    const loaded = [msg(1), msg(2), msg(4, 'me')];
    expect(unreadCount(conv({ last_seq: 4 }), loaded, 'me')).toBe(1);
    expect(unreadCount(conv({ last_seq: 4, last_read_seq: 4 }), loaded, 'me')).toBe(0);
  });

  it('never guesses when the server sent no marker', () => {
    expect(unreadCount(conv({ last_read_seq: undefined }), five, 'me')).toBe(0);
    expect(unreadCount(conv({ last_read_seq: null }), five, 'me')).toBe(0);
  });

  it('caps the badge at 99+', () => {
    expect(unreadBadge(99)).toBe('99');
    expect(unreadBadge(100)).toBe('99+');
  });

  it('moves markers forward only, and never invents one', () => {
    const store = new ChatStore();
    store.addConversation(conv());
    store.addConversation({ id: 'legacy', kind: 'dm', members: [] });
    store.setReadMarker('c', 4);
    store.setReadMarker('c', 3);
    store.setReadMarker('legacy', 9);
    store.setReadMarker('missing', 1);
    expect(store.conversations.find(c => c.id === 'c')?.last_read_seq).toBe(4);
    expect(store.conversations.find(c => c.id === 'legacy')?.last_read_seq).toBeUndefined();
  });
});

describe('unread review regressions', () => {
  const conv = (over: Partial<ChatConversation> = {}): ChatConversation => ({
    id: 'c', kind: 'dm', members: ['me', 'peer'], last_seq: 5, last_read_seq: 5, ...over,
  });

  it('never lets a stale list response rewind a read marker or position', () => {
    const store = new ChatStore();
    store.addConversation(conv());
    store.addConversation(conv({ last_read_seq: 3, last_seq: 4 }));
    expect(store.conversations[0]).toMatchObject({ last_read_seq: 5, last_seq: 5 });
    store.addConversation(conv({ last_read_seq: 7, last_seq: 9 }));
    expect(store.conversations[0]).toMatchObject({ last_read_seq: 7, last_seq: 9 });
  });

  it('records a live event position before history arrives', () => {
    const store = new ChatStore();
    store.addConversation(conv({ last_seq: 2, last_read_seq: 2 }));
    store.applyGatewayEvent({ id: 'e7', topic: 'message.created', payload: { conversation_id: 'c', data: { message_id: 'm7', seq: 7 } } });
    const row = store.conversations.find(c => c.id === 'c')!;
    expect(row.last_seq).toBe(7);
    expect(unreadCount(row, undefined, 'me')).toBe(5);
  });

  it('initializes an absent marker only when told the server supports them', () => {
    const store = new ChatStore();
    store.addConversation({ id: 'n', kind: 'channel', members: [] });
    store.setReadMarker('n', 3);
    expect(store.conversations[0].last_read_seq).toBeUndefined();
    store.setReadMarker('n', 3, true);
    expect(store.conversations[0].last_read_seq).toBe(3);
  });

  it('forgets removed conversations with their history', () => {
    const store = new ChatStore();
    store.addConversation(conv());
    store.addConversation(conv({ id: 'keep' }));
    store.mergeHistoryPage('c', [{ id: 'm1', conversation_id: 'c', sender_id: 'peer', seq: 1, ciphertext_b64: '', client_msg_id: 'x', sent_at: '' }]);
    store.removeConversations(['c']);
    expect(store.conversations.map(c => c.id)).toEqual(['keep']);
    expect(store.messages.has('c')).toBe(false);
    expect(store.historyCursor('c')).toBe(0);
  });
});

