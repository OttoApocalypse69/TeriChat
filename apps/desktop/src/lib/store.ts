// Client-side chat store: conversations + messages with idempotent merge.
// Dedup keys: message `id` and per-conversation `client_msg_id` (the send
// idempotency key). Gateway event ids are tracked separately for resume.

import type { ConversationBody, MessageBody } from './api';
import type { GatewayOutboxEvent } from './gateway';

export interface ChatMessage {
  id: string;
  conversation_id: string;
  sender_id: string;
  seq: number;
  ciphertext_b64: string;
  nonce_b64?: string | null;
  client_msg_id: string;
  sent_at: string;
}

export interface ChatConversation {
  id: string;
  kind: string;
  members: string[];
}

export function toChatMessage(m: MessageBody): ChatMessage {
  return {
    id: m.id,
    conversation_id: m.conversation_id,
    sender_id: m.sender_id,
    seq: m.seq,
    ciphertext_b64: m.ciphertext_b64,
    nonce_b64: m.nonce_b64 ?? null,
    client_msg_id: m.client_msg_id,
    sent_at: m.sent_at,
  };
}

function sortMessages(list: ChatMessage[]): ChatMessage[] {
  return [...list].sort(
    (a, b) =>
      a.seq - b.seq ||
      a.sent_at.localeCompare(b.sent_at) ||
      a.id.localeCompare(b.id),
  );
}

/**
 * Merge incoming messages into existing ones without duplicates.
 * Two rows are the same message when `id` matches OR when
 * (conversation_id, client_msg_id) matches (idempotent retry echo).
 */
export function mergeMessages(
  existing: ChatMessage[],
  incoming: ChatMessage[],
): ChatMessage[] {
  const byId = new Map(existing.map((m) => [m.id, m]));
  const byClient = new Map(
    existing.map((m) => [`${m.conversation_id}:${m.client_msg_id}`, m.id]),
  );
  const out = [...existing];
  for (const m of incoming) {
    if (byId.has(m.id)) continue;
    const ck = `${m.conversation_id}:${m.client_msg_id}`;
    if (byClient.has(ck)) continue;
    byId.set(m.id, m);
    byClient.set(ck, m.id);
    out.push(m);
  }
  return sortMessages(out);
}

export function upsertConversation(
  list: ChatConversation[],
  conv: ConversationBody | ChatConversation,
): ChatConversation[] {
  const next = list.some((c) => c.id === conv.id)
    ? list.map((c) => (c.id === conv.id ? { ...c, ...conv } : c))
    : [...list, { id: conv.id, kind: conv.kind, members: conv.members }];
  return [...next].sort((a, b) => a.id.localeCompare(b.id));
}

export interface GatewayMessageInfo {
  conversationId: string;
  messageId: string;
  seq: number;
  eventId: string;
}

/** Extract routing info from a `message.created` gateway event payload. */
export function gatewayEventInfo(
  event: GatewayOutboxEvent,
): GatewayMessageInfo | null {
  if (event.topic !== 'message.created') return null;
  const p = event.payload as Record<string, unknown>;
  const conversationId =
    typeof p.conversation_id === 'string' ? p.conversation_id : undefined;
  const data = p.data as Record<string, unknown> | undefined;
  const messageId =
    typeof data?.message_id === 'string' ? data.message_id : undefined;
  const seq = typeof data?.seq === 'number' ? data.seq : undefined;
  if (!conversationId || !messageId || seq === undefined) return null;
  return { conversationId, messageId, seq, eventId: event.id };
}

export class ChatStore {
  conversations: ChatConversation[] = [];
  messages = new Map<string, ChatMessage[]>();
  seenEventIds = new Set<string>();
  lastEventId: string | null = null;

  addConversation(conv: ConversationBody | ChatConversation): void {
    this.conversations = upsertConversation(this.conversations, conv);
  }

  ensureConversation(id: string, kind = 'dm'): ChatConversation {
    let found = this.conversations.find((c) => c.id === id);
    if (!found) {
      found = { id, kind, members: [] };
      this.conversations = upsertConversation(this.conversations, found);
    }
    return found;
  }

  mergeHistory(conversationId: string, incoming: ChatMessage[]): ChatMessage[] {
    const merged = mergeMessages(this.messages.get(conversationId) ?? [], incoming);
    this.messages.set(conversationId, merged);
    return merged;
  }

  mergeOutgoing(message: ChatMessage): ChatMessage[] {
    return this.mergeHistory(message.conversation_id, [message]);
  }

  maxSeq(conversationId: string): number {
    const list = this.messages.get(conversationId) ?? [];
    return list.reduce((m, x) => Math.max(m, x.seq), 0);
  }

  /**
   * Record a gateway event for resume/dedup. Returns routing info for new
   * `message.created` events, or null for duplicates / non-message events.
   * The caller fetches full history for the conversation (events carry only
   * ids, never envelope bytes).
   */
  applyGatewayEvent(event: GatewayOutboxEvent): GatewayMessageInfo | null {
    if (this.seenEventIds.has(event.id)) return null; // replay dedup
    this.seenEventIds.add(event.id);
    this.lastEventId = event.id;
    const info = gatewayEventInfo(event);
    if (!info) return null;
    this.ensureConversation(info.conversationId);
    return info;
  }
}
