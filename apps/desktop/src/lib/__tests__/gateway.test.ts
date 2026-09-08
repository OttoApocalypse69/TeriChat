import { describe, expect, it, vi } from 'vitest';
import {
  backoffDelay,
  GatewayClient,
  identifyFrame,
  isDuplicateEvent,
  parseGatewayFrame,
  type GatewayOutboxEvent,
  type WsLike,
} from '../gateway';
import { ChatStore } from '../store';

function mockSocket(): WsLike & {
  sent: string[];
  peerText: (raw: string) => void;
  peerClose: () => void;
} {
  const s = {
    sent: [] as string[],
    onopen: null as WsLike['onopen'],
    onmessage: null as WsLike['onmessage'],
    onclose: null as WsLike['onclose'],
    onerror: null as WsLike['onerror'],
    send(data: string) {
      s.sent.push(data);
    },
    close() {
      s.peerClose();
    },
    peerText(raw: string) {
      s.onmessage?.({ data: raw });
    },
    peerClose() {
      s.onclose?.({});
    },
  };
  return s;
}

describe('reconnect backoff', () => {
  it('grows exponentially and caps', () => {
    expect(backoffDelay(0)).toBe(500);
    expect(backoffDelay(1)).toBe(1000);
    expect(backoffDelay(2)).toBe(2000);
    expect(backoffDelay(3)).toBe(4000);
    expect(backoffDelay(10)).toBe(10_000);
    expect(backoffDelay(100)).toBe(10_000);
  });

  it('identifies with resume_after for resume', () => {
    expect(JSON.parse(identifyFrame(null))).toEqual({
      op: 'identify',
      resume_after: null,
    });
    expect(JSON.parse(identifyFrame('evt-123'))).toEqual({
      op: 'identify',
      resume_after: 'evt-123',
    });
  });
});

describe('gateway frames + resume replay without duplicates', () => {
  it('parses ready / event / heartbeat_ack / error frames', () => {
    expect(
      parseGatewayFrame('{"op":"ready","session_id":"s","user_id":"u"}'),
    ).toEqual({ kind: 'ready', session_id: 's', user_id: 'u' });
    const parsed = parseGatewayFrame(
      '{"op":"event","event":{"event_id":"e1","topic":"message.created","payload":{"conversation_id":"c"}}}',
    );
    expect(parsed).toEqual({
      kind: 'event',
      event: {
        id: 'e1',
        topic: 'message.created',
        payload: { conversation_id: 'c' },
      },
    });
    expect(parseGatewayFrame('{"op":"heartbeat_ack","seq":7}')).toEqual({
      kind: 'heartbeat_ack',
      seq: 7,
    });
    expect(parseGatewayFrame('{"op":"error","code":"bad_frame"}')).toEqual({
      kind: 'error',
      code: 'bad_frame',
    });
    expect(parseGatewayFrame('not json')).toEqual({ kind: 'unknown' });
  });

  it('dedupes replayed events by event id', () => {
    const seen = new Set(['e1']);
    expect(isDuplicateEvent(seen, 'e1')).toBe(true);
    expect(isDuplicateEvent(seen, 'e2')).toBe(false);
    expect(isDuplicateEvent(['e1'], 'e1')).toBe(true);
  });

  it('store resume: replay after resume_after delivers each event once', () => {
    const store = new ChatStore();
    const ev = (id: string, seq: number): GatewayOutboxEvent => ({
      id,
      topic: 'message.created',
      payload: {
        conversation_id: 'conv-1',
        data: { message_id: `m${seq}`, seq },
      },
    });
    // Live delivery of e1..e2, drop, replay of e3 once + duplicate e2.
    expect(store.applyGatewayEvent(ev('e1', 1))?.seq).toBe(1);
    expect(store.applyGatewayEvent(ev('e2', 2))?.seq).toBe(2);
    // Server replays everything after resume_after=e2: e3 new, e2 dup.
    expect(store.applyGatewayEvent(ev('e2', 2))).toBeNull();
    expect(store.applyGatewayEvent(ev('e3', 3))?.seq).toBe(3);
    expect(store.applyGatewayEvent(ev('e3', 3))).toBeNull();
    expect(store.lastEventId).toBe('e3');
  });

  it('bounds transport dedup while notifying opt-in callers of retained duplicates', () => {
    const socket = mockSocket();
    const onEvent = vi.fn();
    const onDuplicateEvent = vi.fn();
    const gw = new GatewayClient({
      httpBase: 'http://127.0.0.1:3001', token: 'synthetic',
      getResumeAfter: () => null, onStatus: () => {},
      onEvent, onDuplicateEvent, wsFactory: () => socket, heartbeatMs: 0,
    });
    const deliver = (id: string) => socket.peerText(JSON.stringify({
      op: 'event', event: { event_id: id, topic: 'message.created', payload: {} },
    }));
    try {
      gw.connect();
      for (let i = 0; i < 1025; i++) deliver(`e${i}`);
      expect(gw.seenEventIds.size).toBe(1024);
      expect(gw.seenEventIds.has('e0')).toBe(false);
      expect(onEvent).toHaveBeenCalledTimes(1025);
      deliver('e1024');
      expect(onDuplicateEvent).toHaveBeenCalledExactlyOnceWith({ id: 'e1024', topic: 'message.created', payload: {} });
      expect(onEvent).toHaveBeenCalledTimes(1025);
      // Eviction permits redelivery; application effects still need their own dedup.
      deliver('e0');
      expect(onEvent).toHaveBeenCalledTimes(1026);
      expect(onDuplicateEvent).toHaveBeenCalledTimes(1);
      expect(gw.seenEventIds.size).toBe(1024);
      gw.close();
      deliver('e0');
      expect(onDuplicateEvent).toHaveBeenCalledTimes(1);
    } finally {
      gw.close();
    }
  });

  it('gateway client sends resume_after on reconnect and ignores redelivery', () => {
    vi.useFakeTimers();
    try {
      const sockets: ReturnType<typeof mockSocket>[] = [];
      const urls: string[] = [];
      const events: GatewayOutboxEvent[] = [];
      const statuses: string[] = [];
      let resume: string | null = null;
      const gw = new GatewayClient({
        httpBase: 'http://127.0.0.1:3001',
        token: 'tok',
        getResumeAfter: () => resume,
        onEvent: (e) => {
          events.push(e);
          resume = e.id;
        },
        onStatus: (s) => statuses.push(s),
        wsFactory: (url) => {
          urls.push(url);
          const s = mockSocket();
          sockets.push(s);
          return s;
        },
        baseMs: 500,
        capMs: 10_000,
        heartbeatMs: 0,
      });
      gw.connect();
      sockets[0].onopen?.({});
      expect(JSON.parse(sockets[0].sent[0])).toEqual({
        op: 'identify',
        resume_after: null,
      });
      sockets[0].peerText('{"op":"ready","session_id":"s","user_id":"u"}');
      const frame =
        '{"op":"event","event":{"event_id":"e1","topic":"message.created",' +
        '"payload":{"conversation_id":"c1","data":{"message_id":"m1","seq":1}}}}';
      sockets[0].peerText(frame);
      sockets[0].peerText(frame); // at-least-once redelivery
      expect(events.map((e) => e.id)).toEqual(['e1']);

      // Drop -> backoff reconnect carries resume_after=e1.
      sockets[0].peerClose();
      expect(statuses).toContain('reconnecting');
      vi.advanceTimersByTime(500);
      expect(sockets).toHaveLength(2);
      sockets[1].onopen?.({});
      expect(JSON.parse(sockets[1].sent[0])).toEqual({
        op: 'identify',
        resume_after: 'e1',
      });
      // Server replays e1 (dup) then new e2.
      sockets[1].peerText(frame);
      const frame2 = frame.replace('"e1"', '"e2"').replace('"seq":1', '"seq":2');
      sockets[1].peerText(frame2);
      expect(events.map((e) => e.id)).toEqual(['e1', 'e2']);
      gw.close();
    } finally {
      vi.useRealTimers();
    }
  });
});
