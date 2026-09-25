// Client-side chat store: conversations + messages with idempotent merge.
// Dedup keys: message `id` and per-conversation `client_msg_id` (the send
// idempotency key). Gateway event ids are tracked separately for resume.

import type { ConversationBody, MemberProfileBody, MessageBody } from './api';
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
  /** Two-member DM peer handle, resolved server-side (null otherwise). */
  peer_handle?: string | null;
  /** Two-member DM peer display name, resolved server-side. */
  peer_display_name?: string | null;
  /** Highest sent seq, when the server reported message activity. */
  last_seq?: number | null;
  /** `sent_at` of that last message, when the server reported activity. */
  last_sent_at?: string | null;
  /** DM/group roster with handles; absent from older servers and channels. */
  member_profiles?: MemberProfileBody[];
  /** The caller's private read marker; absent when the server predates it. */
  last_read_seq?: number | null;
}

/**
 * Messages from other people after the caller's read marker. Unknown markers
 * (older servers) count as zero rather than guessing. Every position after
 * the marker that history has not loaded still counts, including gaps below
 * a loaded message (e.g. a peer's message whose fetch failed just before
 * the caller's own reply loaded).
 */
export function unreadCount(
  conv: ChatConversation,
  messages: ChatMessage[] | undefined,
  meId: string,
): number {
  const read = conv.last_read_seq;
  if (read == null) return 0;
  const after = (messages ?? []).filter(m => m.seq > read);
  const latest = after.reduce((max, m) => Math.max(max, m.seq), Math.max(conv.last_seq ?? 0, read));
  const fromOthers = after.filter(m => m.sender_id !== meId).length;
  const missing = Math.max(0, latest - read - new Set(after.map(m => m.seq)).size);
  return fromOthers + missing;
}

/** Badge text: exact up to 99. */
export function unreadBadge(count: number): string {
  return count > 99 ? '99+' : String(count);
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
  // Spread the whole row: summary entries carry peer/last-message fields
  // that must survive reload-driven list merges, not just id/kind/members.
  // Positions only move forward: a list response captured before a newer
  // read or event must not rewind them.
  const next = list.some((c) => c.id === conv.id)
    ? list.map((c) => {
      if (c.id !== conv.id) return c;
      const merged: ChatConversation = { ...c, ...conv };
      const incoming = conv as ChatConversation;
      if (c.last_read_seq != null && incoming.last_read_seq != null) {
        merged.last_read_seq = Math.max(c.last_read_seq, incoming.last_read_seq);
      }
      if (c.last_seq != null && incoming.last_seq != null) {
        merged.last_seq = Math.max(c.last_seq, incoming.last_seq);
      }
      return merged;
    })
    : [...list, { ...(conv as ChatConversation) }];
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

/** Primary row title: peer name for DMs, never a raw UUID. */
export function conversationLabel(conv: ChatConversation, meId?: string): string {
  if (conv.kind === 'dm') {
    if (conv.peer_display_name) return conv.peer_display_name;
    if (conv.peer_handle) return `@${conv.peer_handle}`;
    return 'Direct message';
  }
  if (conv.kind === 'channel') return 'Channel';
  if (conv.kind === 'group') {
    // Groups are named by their other members once the roster is known.
    const others = (conv.member_profiles ?? []).filter(p => p.user_id !== meId).map(profileName);
    if (others.length > 0) {
      const shown = others.slice(0, 3).join(', ');
      return others.length > 3 ? `${shown} +${others.length - 3}` : shown;
    }
    return conv.members.length > 0
      ? `Group · ${conv.members.length} members`
      : 'Group';
  }
  return conv.kind || 'Conversation';
}

/** Secondary row line: the handle behind a DM name, or a named group's size. */
export function conversationSublabel(conv: ChatConversation): string | null {
  if (conv.kind === 'dm' && conv.peer_display_name && conv.peer_handle) {
    return `@${conv.peer_handle}`;
  }
  if (conv.kind === 'group' && (conv.member_profiles?.length ?? 0) > 0) {
    return `${conv.members.length} members`;
  }
  return null;
}

function profileName(profile: MemberProfileBody): string {
  return profile.display_name.trim() || `@${profile.handle}`;
}

/** The roster entry for a sender, when the server resolved one. */
export function memberProfile(conv: ChatConversation | null, userId: string): MemberProfileBody | null {
  return conv?.member_profiles?.find(p => p.user_id === userId) ?? null;
}

/** Other people a single request may start a group with (server-enforced). */
export const MAX_GROUP_MEMBERS = 50;

/**
 * Parse group member input ("@ana, mary jane") into unique handles, dropping
 * the caller's own handle (the server adds the creator anyway). Handles may
 * contain spaces, so only commas and new lines separate them; one leading
 * "@" is the display prefix. Handles are case-insensitive server-side.
 */
export function parseMemberHandles(input: string, ownHandle?: string): string[] {
  const own = ownHandle?.trim().toLowerCase();
  const seen = new Set<string>();
  for (const raw of input.split(/[,\n]+/)) {
    const handle = raw.trim().replace(/^@/, '').trim().toLowerCase();
    if (handle && handle !== own) seen.add(handle);
  }
  return [...seen];
}

/** Single avatar letter from the peer name (or kind fallback). */
export function avatarInitial(conv: ChatConversation): string {
  const src =
    conv.peer_display_name?.trim() ||
    conv.peer_handle?.trim() ||
    conv.kind.trim();
  return (src.charAt(0) || '?').toUpperCase();
}

/** Sender tag under a message: `you` for self, else the DM peer or roster name. */
export function senderLabel(
  meId: string,
  senderId: string,
  conv: ChatConversation | null,
): string {
  if (senderId === meId) return 'you';
  if (conv?.kind === 'dm') {
    if (conv.peer_display_name) return conv.peer_display_name;
    if (conv.peer_handle) return `@${conv.peer_handle}`;
  }
  const profile = memberProfile(conv, senderId);
  if (profile) return profileName(profile);
  return senderId.slice(0, 8);
}

/** `YYYY-MM-DD` (UTC) day bucket for divider grouping; '' when invalid. */
export function dayKey(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? '' : d.toISOString().slice(0, 10);
}

/** Divider text: Today / Yesterday / YYYY-MM-DD. */
export function dayLabel(iso: string, nowIso?: string): string {
  const key = dayKey(iso);
  if (!key) return '';
  const today = dayKey(nowIso ?? new Date().toISOString());
  if (key === today) return 'Today';
  const base = new Date(`${today}T00:00:00Z`).getTime();
  const yesterday = new Date(base - 86_400_000).toISOString().slice(0, 10);
  if (key === yesterday) return 'Yesterday';
  return key;
}

/** Local `HH:MM` clock time from `sent_at`; '' when missing/invalid. */
export function formatClockTime(iso: string | null | undefined): string {
  if (!iso) return '';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
}

/**
 * List-row timestamp: clock time for today's activity, `Yesterday` for
 * yesterday's, otherwise the `YYYY-MM-DD` day.
 */
export function formatListTime(
  iso: string | null | undefined,
  nowIso?: string,
): string {
  if (!iso) return '';
  const label = dayLabel(iso, nowIso);
  if (label === 'Today') return formatClockTime(iso);
  return label;
}

/** Single-line list preview, capped at `max` chars with an ellipsis. */
export function truncatePreview(text: string, max = 60): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  if (flat.length <= max) return flat;
  return `${flat.slice(0, max - 1)}…`;
}

/**
 * Last activity for sorting/previews: newest loaded message wins, falling
 * back to the server-reported `last_sent_at` (covers quiet reloads).
 */
export function lastActivityAt(
  conv: ChatConversation,
  messages?: ChatMessage[],
): string | null {
  if (messages && messages.length > 0) {
    return messages[messages.length - 1].sent_at;
  }
  return conv.last_sent_at ?? null;
}

/** Newest-activity-first ordering for the conversation list. */
export function sortConversations(
  list: ChatConversation[],
  messagesByConversation?: Map<string, ChatMessage[]>,
): ChatConversation[] {
  return [...list].sort(
    (a, b) =>
      (lastActivityAt(b, messagesByConversation?.get(b.id)) ?? '').localeCompare(
        lastActivityAt(a, messagesByConversation?.get(a.id)) ?? '',
      ) || a.id.localeCompare(b.id),
  );
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

  // Only completed, ordered history pages advance this cursor. An outgoing
  // response may be arbitrarily far ahead of the last fetched page.
  historySeq = new Map<string, number>();

  historyCursor(conversationId: string): number {
    return this.historySeq.get(conversationId) ?? 0;
  }

  mergeHistoryPage(conversationId: string, incoming: ChatMessage[]): void {
    this.mergeHistory(conversationId, incoming);
    const cursor = incoming.reduce(
      (seq, m) => Math.max(seq, m.seq),
      this.historyCursor(conversationId),
    );
    this.historySeq.set(conversationId, cursor);
  }

  /** Advance the caller's read marker; markers never move backwards. */
  /**
   * Advance the caller's read marker; markers never move backwards. An absent
   * marker is only created when `initialize` says the server supports them
   * (e.g. a channel added this session, answered by the read endpoint).
   */
  setReadMarker(conversationId: string, seq: number, initialize = false): void {
    const conv = this.conversations.find(c => c.id === conversationId);
    if (!conv) return;
    if (conv.last_read_seq == null ? !initialize : seq <= conv.last_read_seq) return;
    this.conversations = upsertConversation(this.conversations, { ...conv, last_read_seq: seq });
  }

  /** Forget conversations the caller can no longer reach (e.g. a left workspace). */
  removeConversations(ids: Iterable<string>): void {
    const gone = new Set(ids);
    if (gone.size === 0) return;
    this.conversations = this.conversations.filter(c => !gone.has(c.id));
    for (const id of gone) {
      this.messages.delete(id);
      this.historySeq.delete(id);
    }
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
    const conv = this.ensureConversation(info.conversationId);
    // Record the position now, so unread state survives a failed or slow
    // history fetch for the message this event announces.
    if ((conv.last_seq ?? 0) < info.seq) {
      this.conversations = upsertConversation(this.conversations, { ...conv, last_seq: info.seq });
    }
    return info;
  }
}
