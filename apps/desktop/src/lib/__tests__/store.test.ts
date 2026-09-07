import { describe, expect, it } from 'vitest';
import {
  ChatStore,
  gatewayEventInfo,
  mergeMessages,
  upsertConversation,
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
