// Realtime gateway client: WebSocket with identify / resume / reconnect.
// Wire protocol (server contract): first send
//   {"op":"identify","resume_after":"<event-uuid>"|null}
// server replies {"op":"ready",...}, replays missed
// {"op":"event","event":{"event_id","topic","payload"}} frames, then streams
// live ones. Delivery is at-least-once: callers MUST dedup by event id.
// Ephemeral {"op":"typing","conversation_id","user_id","last_seq"} frames carry
// no event id, are never replayed and never touch dedup or the resume position.
// `last_seq` is the conversation's highest message seq when the signal was
// sent, so a signal overtaken by the typist's next message can be dropped.

export type GatewayStatus =
  | 'disconnected'
  | 'connecting'
  | 'connected'
  | 'reconnecting';

export interface GatewayOutboxEvent {
  /** Canonical event id (server serializes OutboxEntry.id as `event_id`). */
  id: string;
  topic: string;
  payload: Record<string, unknown>;
}

export type ParsedGatewayFrame =
  | { kind: 'ready'; session_id: string; user_id: string }
  | { kind: 'event'; event: GatewayOutboxEvent }
  | { kind: 'heartbeat_ack'; seq: number }
  | { kind: 'error'; code: string }
  | { kind: 'typing'; conversation_id: string; user_id: string; last_seq: number }
  | { kind: 'unknown' };

/** Exponential backoff: baseMs * 2^attempt, capped at capMs. Pure/tested. */
export function backoffDelay(
  attempt: number,
  baseMs = 500,
  capMs = 10_000,
): number {
  const safe = Math.max(0, Math.floor(attempt));
  return Math.min(capMs, baseMs * 2 ** safe);
}

/** Parse one server text frame into a typed union. Never throws. */
export function parseGatewayFrame(raw: string): ParsedGatewayFrame {
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return { kind: 'unknown' };
  }
  if (typeof v !== 'object' || v === null) return { kind: 'unknown' };
  const op = (v as Record<string, unknown>).op;
  const obj = v as Record<string, unknown>;
  if (op === 'ready') {
    if (typeof obj.session_id === 'string' && typeof obj.user_id === 'string') {
      return {
        kind: 'ready',
        session_id: obj.session_id,
        user_id: obj.user_id,
      };
    }
    return { kind: 'unknown' };
  }
  if (op === 'event') {
    const e = obj.event as Record<string, unknown> | undefined;
    const eventId =
      typeof e?.event_id === 'string'
        ? (e.event_id as string)
        : typeof e?.id === 'string'
          ? (e.id as string)
          : undefined;
    if (e && typeof eventId === 'string' && typeof e.topic === 'string') {
      return {
        kind: 'event',
        event: {
          id: eventId,
          topic: e.topic as string,
          payload:
            typeof e.payload === 'object' && e.payload !== null
              ? (e.payload as Record<string, unknown>)
              : {},
        },
      };
    }
    return { kind: 'unknown' };
  }
  if (op === 'heartbeat_ack') {
    return {
      kind: 'heartbeat_ack',
      seq: typeof obj.seq === 'number' ? obj.seq : Number(obj.seq) || 0,
    };
  }
  if (op === 'error') {
    return {
      kind: 'error',
      code: typeof obj.code === 'string' ? obj.code : 'unknown',
    };
  }
  if (op === 'typing') {
    if (
      typeof obj.conversation_id === 'string'
      && typeof obj.user_id === 'string'
      && Number.isSafeInteger(obj.last_seq)
      && (obj.last_seq as number) >= 0
    ) {
      return {
        kind: 'typing',
        conversation_id: obj.conversation_id,
        user_id: obj.user_id,
        last_seq: obj.last_seq as number,
      };
    }
    return { kind: 'unknown' };
  }
  return { kind: 'unknown' };
}

/** True when eventId was already processed (client-side at-least-once dedup). */
export function isDuplicateEvent(
  seen: Set<string> | string[],
  eventId: string,
): boolean {
  return seen instanceof Set ? seen.has(eventId) : seen.includes(eventId);
}

export function gatewayWsUrl(
  httpBase: string,
  token: string,
  resumeAfter: string | null,
): string {
  const base = httpBase.replace(/\/$/, '');
  const wsBase =
    base.startsWith('https://')
      ? `wss://${base.slice('https://'.length)}`
      : base.startsWith('http://')
        ? `ws://${base.slice('http://'.length)}`
        : base;
  const q = new URLSearchParams({ token });
  void resumeAfter;
  return `${wsBase}/v1/gateway?${q.toString()}`;
}

export function identifyFrame(resumeAfter: string | null): string {
  return JSON.stringify({ op: 'identify', resume_after: resumeAfter });
}

/** Minimal socket surface so tests can inject a mock. */
export interface WsLike {
  send(data: string): void;
  close(): void;
  onopen: ((ev?: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev?: unknown) => void) | null;
  onerror: ((ev?: unknown) => void) | null;
}

export type WsFactory = (url: string) => WsLike;

const defaultWsFactory: WsFactory = (url) =>
  new WebSocket(url) as unknown as WsLike;

export interface GatewayOptions {
  httpBase: string;
  token: string;
  getResumeAfter: () => string | null;
  onEvent: (event: GatewayOutboxEvent) => void;
  /** Redelivery is not proof that application effects completed. Optional retry
   * notification; onEvent retains its existing transport-deduplicated contract. */
  onDuplicateEvent?: (event: GatewayOutboxEvent) => void;
  /** Someone else is typing; best effort, never deduplicated or replayed. */
  onTyping?: (signal: { conversationId: string; userId: string; lastSeq: number }) => void;
  onStatus: (status: GatewayStatus) => void;
  wsFactory?: WsFactory;
  baseMs?: number;
  capMs?: number;
  heartbeatMs?: number;
}

/**
 * Persistent gateway connection with resume + exponential-backoff reconnect.
 * Last processed event id is read via getResumeAfter on every (re)connect,
 * so resume position survives drops.
 */
export class GatewayClient {
  private opts: GatewayOptions;
  private ws: WsLike | null = null;
  private closed = true;
  private attempt = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private heartTimer: ReturnType<typeof setInterval> | null = null;
  private heartSeq = 0;
  /** Bounded transport cache; application state owns durable-effect dedup. */
  readonly seenEventIds = new Set<string>();

  constructor(opts: GatewayOptions) {
    this.opts = opts;
  }

  get reconnectAttempt(): number {
    return this.attempt;
  }

  connect(): void {
    this.closed = false;
    this.openSocket();
  }

  close(): void {
    this.closed = true;
    this.attempt = 0;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.stopHeartbeat();
    try {
      this.ws?.close();
    } catch {
      // Ignore close errors on teardown.
    }
    this.ws = null;
    this.opts.onStatus('disconnected');
  }

  private openSocket(): void {
    if (this.closed) return;
    const resumeAfter = this.opts.getResumeAfter();
    const url = gatewayWsUrl(this.opts.httpBase, this.opts.token, resumeAfter);
    this.opts.onStatus(this.attempt === 0 ? 'connecting' : 'reconnecting');
    const factory = this.opts.wsFactory ?? defaultWsFactory;
    const ws = factory(url);
    this.ws = ws;
    ws.onopen = () => {
      if (this.closed || this.ws !== ws) return;
      ws.send(identifyFrame(resumeAfter));
      this.startHeartbeat(ws);
    };
    ws.onmessage = (ev) => {
      if (this.closed || this.ws !== ws || typeof ev.data !== 'string') return;
      const frame = parseGatewayFrame(ev.data);
      if (frame.kind === 'ready') {
        this.attempt = 0;
        this.opts.onStatus('connected');
      } else if (frame.kind === 'event') {
        if (this.seenEventIds.has(frame.event.id)) {
          this.opts.onDuplicateEvent?.(frame.event);
          return;
        }
        this.seenEventIds.add(frame.event.id);
        if (this.seenEventIds.size > 1024) {
          this.seenEventIds.delete(this.seenEventIds.values().next().value!);
        }
        this.opts.onEvent(frame.event);
      } else if (frame.kind === 'typing') {
        this.opts.onTyping?.({
          conversationId: frame.conversation_id,
          userId: frame.user_id,
          lastSeq: frame.last_seq,
        });
      }
      // heartbeat_ack / error / unknown: stay connected, nothing to do.
    };
    const schedule = () => {
      if (this.closed || this.ws !== ws) return;
      this.stopHeartbeat();
      this.ws = null;
      const delay = backoffDelay(
        this.attempt,
        this.opts.baseMs ?? 500,
        this.opts.capMs ?? 10_000,
      );
      this.attempt += 1;
      this.opts.onStatus('reconnecting');
      this.timer = setTimeout(() => this.openSocket(), delay);
    };
    ws.onclose = schedule;
    ws.onerror = schedule;
  }

  private startHeartbeat(ws: WsLike): void {
    this.stopHeartbeat();
    const ms = this.opts.heartbeatMs ?? 15_000;
    if (ms <= 0) return;
    this.heartTimer = setInterval(() => {
      if (this.closed || this.ws !== ws) return;
      this.heartSeq += 1;
      try {
        ws.send(JSON.stringify({ op: 'heartbeat', seq: this.heartSeq }));
      } catch {
        // Send failure surfaces via onclose/onerror -> reconnect.
      }
    }, ms);
  }

  private stopHeartbeat(): void {
    if (this.heartTimer) clearInterval(this.heartTimer);
    this.heartTimer = null;
  }
}
