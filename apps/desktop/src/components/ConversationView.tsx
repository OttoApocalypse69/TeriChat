import { useEffect, useRef, useState } from 'react';
import { decodeOpaqueText } from '../lib/api';
import type { GatewayStatus } from '../lib/gateway';
import {
  avatarInitial,
  conversationLabel,
  conversationSublabel,
  dayKey,
  dayLabel,
  formatClockTime,
  senderLabel,
  type ChatConversation,
  type ChatMessage,
} from '../lib/store';
import ConnectionIndicator from './ConnectionIndicator';

interface Props {
  conversation: ChatConversation | null;
  messages: ChatMessage[];
  meId: string;
  status: GatewayStatus;
  loading: boolean;
  sending: boolean;
  error: string | null;
  onSend: (text: string) => Promise<void>;
  title?: string | null;
}

export default function ConversationView({
  conversation,
  messages,
  meId,
  status,
  loading,
  sending,
  error,
  onSend,
  title,
}: Props) {
  const [draft, setDraft] = useState('');
  const [sendError, setSendError] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Switching conversations (DM <-> DM, DM <-> channel, channel <-> channel)
  // must never leak the previous composer draft into the new conversation.
  const conversationId = conversation?.id ?? null;
  useEffect(() => {
    setDraft('');
    setSendError(null);
  }, [conversationId]);

  // Real-chat-app behavior: follow the tail as new messages arrive.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length, conversationId]);

  async function submit(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    if (!draft.trim()) return;
    setSendError(null);
    try {
      await onSend(draft.trim());
      setDraft('');
    } catch (err) {
      setSendError(err instanceof Error ? err.message : 'send failed');
    }
  }

  if (!conversation) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-zinc-500">
        Select a conversation.
      </div>
    );
  }

  const heading = title ?? conversationLabel(conversation);
  const sub = title ? conversation.kind : conversationSublabel(conversation);

  let lastDay = '';
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b border-zinc-800 px-3 py-2">
        <span
          aria-hidden
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-zinc-700 text-xs font-bold text-zinc-200"
        >
          {avatarInitial(conversation)}
        </span>
        <span className="min-w-0">
          <span className="block truncate text-sm font-semibold text-zinc-100">
            {heading}
          </span>
          {sub && (
            <span className="block truncate text-[11px] text-zinc-500">
              {sub}
            </span>
          )}
        </span>
        <span className="ml-auto shrink-0">
          <ConnectionIndicator status={status} />
        </span>
      </div>
      <p className="border-b border-zinc-800/60 bg-zinc-900/40 px-3 py-1 text-[11px] text-zinc-500">
        Alpha demo: envelopes carry demo plaintext — not end-to-end encrypted.
      </p>
      <div ref={scrollRef} className="flex-1 space-y-1 overflow-y-auto p-3">
        {loading && <p className="text-xs text-zinc-500">Loading history…</p>}
        {messages.map((m) => {
          const divider =
            dayKey(m.sent_at) !== lastDay ? dayLabel(m.sent_at) : null;
          lastDay = dayKey(m.sent_at);
          const mine = m.sender_id === meId;
          return (
            <div key={m.id}>
              {divider && (
                <p className="py-1 text-center text-[11px] font-medium text-zinc-500">
                  — {divider} —
                </p>
              )}
              <div
                className={`max-w-[80%] rounded px-2 py-1 text-sm ${
                  mine
                    ? 'ml-auto bg-emerald-900 text-emerald-50'
                    : 'bg-zinc-800 text-zinc-100'
                }`}
              >
                <p className="whitespace-pre-wrap break-words">
                  {decodeOpaqueText(m.ciphertext_b64)}
                </p>
                <p className="mt-0.5 text-[10px] opacity-60">
                  #{m.seq} {senderLabel(meId, m.sender_id, conversation)}
                  {formatClockTime(m.sent_at)
                    ? ` · ${formatClockTime(m.sent_at)}`
                    : ''}
                </p>
              </div>
            </div>
          );
        })}
        {!loading && messages.length === 0 && (
          <p className="text-xs text-zinc-500">
            No messages yet — say bro.
          </p>
        )}
      </div>
      {error && <p className="px-3 text-xs text-red-400">{error}</p>}
      {sendError && <p className="px-3 text-xs text-red-400">{sendError}</p>}
      <form onSubmit={submit} className="flex gap-2 border-t border-zinc-800 p-2">
        <input
          className="flex-1 rounded bg-zinc-800 px-2 py-1.5 text-sm"
          placeholder="Message (demo plaintext → opaque envelope)"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <button
          type="submit"
          disabled={sending || !draft.trim()}
          className="rounded bg-emerald-600 px-3 py-1.5 text-sm font-semibold disabled:opacity-40"
        >
          {sending ? '…' : 'Send'}
        </button>
      </form>
    </div>
  );
}
