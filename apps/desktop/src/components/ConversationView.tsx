import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
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
  isActivePane?: boolean;
  headerActions?: ReactNode;
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
  isActivePane = true,
  headerActions,
}: Props) {
  const [draft, setDraft] = useState('');
  const draftRevision = useRef(0);
  const [sendError, setSendError] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Switching conversations (DM <-> DM, DM <-> channel, channel <-> channel)
  // must never leak the previous composer draft into the new conversation.
  const conversationId = conversation?.id ?? null;
  useEffect(() => {
    setDraft('');
    setSendError(null);
  }, [conversationId]);

  // Hidden mounted panes have no scroll box. Reconcile pending follows when
  // navigation or CSS breakpoint layout reveals this conversation, not on focus.
  const pendingTail = useRef(false);
  useLayoutEffect(() => {
    pendingTail.current = true;
  }, [messages.length, conversationId]);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const followPendingTail = () => {
      if (pendingTail.current && el.clientHeight > 0) {
        el.scrollTop = el.scrollHeight;
        pendingTail.current = false;
      }
    };
    followPendingTail();
    // ResizeObserver also fires when display:none becomes a real layout box.
    // Consumed pending state leaves readers alone on ordinary viewport resizes.
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(followPendingTail);
    observer.observe(el);
    return () => observer.disconnect();
  }, [messages.length, conversationId, isActivePane]);

  async function submit(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    if (sending || !draft.trim()) return;
    const revision = draftRevision.current;
    setSendError(null);
    try {
      await onSend(draft.trim());
      if (draftRevision.current === revision) setDraft('');
    } catch (err) {
      setSendError(err instanceof Error ? err.message : 'send failed');
    }
  }

  if (!conversation) {
    return (
      <>
      {headerActions}
      <div className="conversation-empty flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-sm text-zinc-400">
        <span className="empty-mark" aria-hidden>UC</span>
        <span className="eyebrow">A little closer, wherever you are</span>
        <span className="text-lg font-semibold text-zinc-100">Your conversations, in one place</span>
        <span>Select a conversation or open a DM to start chatting.</span>
      </div>
      </>
    );
  }

  const heading = title ?? conversationLabel(conversation);
  const sub = title ? conversation.kind : conversationSublabel(conversation);

  let lastDay = '';
  return (
    <div className="conversation-view flex min-h-0 flex-1 flex-col">
      <div className="conversation-header flex shrink-0 items-center gap-3 border-b border-zinc-800 px-5 py-4">
        <span
          aria-hidden
          className="conversation-avatar"
        >
          {conversation.kind === 'channel' ? '#' : avatarInitial(conversation)}
        </span>
        <span className="min-w-0 flex-1">
          <h2 className="block truncate text-lg font-semibold text-zinc-100">
            {heading}
          </h2>
          {sub && (
            <span className="block truncate text-[11px] text-zinc-500">
              {sub}
            </span>
          )}
        </span>
        <span className="ml-auto shrink-0">
          <ConnectionIndicator status={status} />
        </span>
        {headerActions}
      </div>
      <p className="plaintext-warning shrink-0 border-b border-amber-900/40 bg-amber-950/20 px-5 py-2 text-xs leading-relaxed text-amber-200/90">
        Alpha demo: envelopes carry demo plaintext — not end-to-end encrypted.
      </p>
      <div ref={scrollRef} className="message-history min-h-0 flex-1 space-y-3 overflow-y-auto p-5" aria-label="Message history">
        <div className="conversation-intro">
          <span className="intro-symbol" aria-hidden>{conversation.kind === 'channel' ? '#' : '@'}</span>
          <h3>{conversation.kind === 'channel' ? `Welcome to ${heading}` : `Your conversation with ${heading}`}</h3>
          <p>{conversation.kind === 'channel' ? 'A shared space for this workspace.' : 'Your direct messages, together in one place.'}</p>
        </div>
        {loading && <p className="text-sm text-zinc-400">Loading history…</p>}
        {messages.map((m) => {
          const divider =
            dayKey(m.sent_at) !== lastDay ? dayLabel(m.sent_at) : null;
          lastDay = dayKey(m.sent_at);
          const mine = m.sender_id === meId;
          return (
            <div key={m.id}>
              {divider && (
                <p className="day-divider">
                  <span>{divider}</span>
                </p>
              )}
              <div className={`message-row ${mine ? 'message-row-own' : ''}`}>
                <span className="message-avatar" aria-hidden>{mine ? 'Y' : senderLabel(meId, m.sender_id, conversation).slice(0, 1).toUpperCase()}</span>
                <div className="message-bubble">
                <p className="message-meta">
                  <strong>{senderLabel(meId, m.sender_id, conversation)}</strong>
                  <time dateTime={m.sent_at ?? undefined}>{formatClockTime(m.sent_at)}</time>
                  <span className="message-sequence">#{m.seq}</span>
                </p>
                <p className="message-text whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
                  {decodeOpaqueText(m.ciphertext_b64)}
                </p>
                </div>
              </div>
            </div>
          );
        })}
        {!loading && messages.length === 0 && (
          <p className="text-sm text-zinc-400">
            No messages yet — say bro.
          </p>
        )}
      </div>
      {error && <p role="alert" className="px-4 py-1 text-sm text-red-400">{error}</p>}
      {sendError && <p role="alert" className="px-4 py-1 text-sm text-red-400">{sendError}</p>}
      <form onSubmit={submit} className="message-composer">
        <div className="composer-field">
        <input
          aria-label="Message"
          className="min-w-0 flex-1"
          placeholder="Message (demo plaintext → opaque envelope)"
          value={draft}
          onChange={(e) => {
            draftRevision.current += 1;
            setDraft(e.target.value);
          }}
        />
        <button
          type="submit"
          disabled={sending || !draft.trim()}
          className="send-button disabled:opacity-40"
        >
          {sending ? '…' : 'Send'}
        </button>
        </div>
        <p className="composer-hint"><span>Enter to send</span><span>Alpha demo · plaintext messages</span></p>
      </form>
    </div>
  );
}
