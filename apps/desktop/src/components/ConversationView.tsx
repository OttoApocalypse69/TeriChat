import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { decodeOpaqueText } from '../lib/api';
import { avatarGradient } from '../lib/avatar';
import {
  avatarInitial,
  conversationLabel,
  conversationSublabel,
  dayKey,
  dayLabel,
  formatClockTime,
  memberProfile,
  senderLabel,
  type ChatConversation,
  type ChatMessage,
} from '../lib/store';
import { BrandMark, OpenLockIcon, PeopleIcon, SendIcon } from './icons';

interface Props {
  conversation: ChatConversation | null;
  messages: ChatMessage[];
  meId: string;
  meHandle?: string;
  loading: boolean;
  sending: boolean;
  error: string | null;
  onSend: (text: string) => Promise<void>;
  title?: string | null;
  isActivePane?: boolean;
  headerActions?: ReactNode;
  /** Read marker when this conversation opened; later messages are new. */
  unreadAfterSeq?: number | null;
}

// Same author within five minutes reads as one block: no repeated header.
const GROUP_WINDOW_MS = 5 * 60 * 1000;
function continuesGroup(prev: ChatMessage | undefined, next: ChatMessage): boolean {
  if (!prev || prev.sender_id !== next.sender_id || !prev.sent_at || !next.sent_at) return false;
  if (dayKey(prev.sent_at) !== dayKey(next.sent_at)) return false;
  const gap = new Date(next.sent_at).getTime() - new Date(prev.sent_at).getTime();
  return gap >= 0 && gap <= GROUP_WINDOW_MS;
}

export default function ConversationView({
  conversation,
  messages,
  meId,
  meHandle,
  loading,
  sending,
  error,
  onSend,
  title,
  isActivePane = true,
  headerActions,
  unreadAfterSeq = null,
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
      <div className="conversation-empty">
        <span className="empty-mark" aria-hidden><BrandMark size={32} /></span>
        <span className="label-mono">A little closer, wherever you are</span>
        <h2>Your conversations, in one place</h2>
        <p>Select a conversation or open a DM to start chatting.</p>
      </div>
      </>
    );
  }

  const heading = title ?? conversationLabel(conversation, meId);
  const sub = title ? conversation.kind : conversationSublabel(conversation);
  const isChannel = conversation.kind === 'channel';
  const isGroup = conversation.kind === 'group';
  // One colour per person across the list, header and history: key by handle.
  const personKey = (senderId: string) => senderId === meId
    ? (meHandle ?? meId)
    : memberProfile(conversation, senderId)?.handle
      ?? (conversation.kind === 'dm' && conversation.peer_handle ? conversation.peer_handle : senderId);
  const peerKey = conversation.peer_handle ?? conversation.id;

  let lastDay = '';
  // The first message from someone else after the opening marker.
  const firstNewId = unreadAfterSeq == null
    ? null
    : messages.find(m => m.seq > unreadAfterSeq && m.sender_id !== meId)?.id ?? null;
  return (
    <div className="conversation-view flex min-h-0 flex-1 flex-col">
      <div className="conversation-header">
        <span className="conversation-title">
          {isGroup && <span aria-hidden className="tile h-7 w-7"><PeopleIcon size={15} /></span>}
          {!isChannel && !isGroup && <span aria-hidden className="avatar avatar-28" style={{ background: avatarGradient(peerKey) }}>{avatarInitial(conversation)}</span>}
          <span className="conversation-title-text">
            <h2>
              {/* Channel titles keep their literal "#name" text; the hash is styled as the glyph. */}
              {isChannel && heading.startsWith('#') ? <><span className="channel-glyph">#</span>{heading.slice(1)}</> : heading}
            </h2>
            {sub && (
              <span className="meta-mono truncate">
                {sub}
              </span>
            )}
          </span>
        </span>
        {headerActions}
      </div>
      <div className="trust-strip">
        <OpenLockIcon />
        <p className="plaintext-warning">
          Alpha demo: envelopes carry demo plaintext — not end-to-end encrypted.
        </p>
      </div>
      <div ref={scrollRef} className="message-history" aria-label="Message history">
        <div className="conversation-intro">
          {isChannel
            ? <span className="tile h-10 w-10 font-display text-xl" aria-hidden>#</span>
            : isGroup
              ? <span className="tile h-10 w-10" aria-hidden><PeopleIcon size={20} /></span>
              : <span className="avatar h-10 w-10 text-[15px]" aria-hidden style={{ background: avatarGradient(peerKey) }}>{avatarInitial(conversation)}</span>}
          <h3>{isChannel ? `Welcome to ${heading}` : isGroup ? `Your group with ${heading}` : `Your conversation with ${heading}`}</h3>
          <p>{isChannel ? 'A shared space for this workspace.' : isGroup ? 'Everyone here sees every message in this group.' : 'Your direct messages, together in one place.'}</p>
        </div>
        {loading && <p className="history-note meta-mono">Loading history…</p>}
        {messages.map((m, index) => {
          const divider =
            dayKey(m.sent_at) !== lastDay ? dayLabel(m.sent_at) : null;
          lastDay = dayKey(m.sent_at);
          const mine = m.sender_id === meId;
          const author = senderLabel(meId, m.sender_id, conversation);
          const isFirstNew = m.id === firstNewId;
          const follow = !divider && !isFirstNew && continuesGroup(messages[index - 1], m);
          const clock = formatClockTime(m.sent_at);
          return (
            <div key={m.id}>
              {divider && (
                <p className="day-divider">
                  <span>{divider}</span>
                </p>
              )}
              {isFirstNew && (
                <div className="new-divider" role="separator" aria-label="New messages">
                  <span aria-hidden>NEW</span>
                </div>
              )}
              <div className={`message-row${mine ? ' message-row-own' : ''}${follow ? ' message-row--follow' : ''}`}>
                <div className="message-gutter">
                  {follow
                    ? <span className="message-gutter-time font-mono" aria-hidden>{clock}</span>
                    : <span className="avatar avatar-32" aria-hidden style={{ background: avatarGradient(personKey(m.sender_id)) }}>
                      {mine ? (meHandle?.slice(0, 1).toUpperCase() || 'Y') : author.replace(/^@/, '').slice(0, 1).toUpperCase()}
                    </span>}
                </div>
                <div className="message-bubble">
                  {/* Follow-ups keep author, time and sequence for assistive tech and tooling. */}
                  <p className={follow ? 'message-meta sr-only' : 'message-meta'}>
                    <strong>{author}</strong>
                    <time dateTime={m.sent_at ?? undefined}>{clock}</time>
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
          <p className="history-note">
            No messages yet — say bro.
          </p>
        )}
      </div>
      {error && <p role="alert" className="stage-alert text-alert">{error}</p>}
      {sendError && <p role="alert" className="stage-alert text-alert">{sendError}</p>}
      <form onSubmit={submit} className="message-composer">
        <div className="composer-field">
          <input
            aria-label="Message"
            placeholder="Message (demo plaintext → opaque envelope)"
            value={draft}
            onChange={(e) => {
              draftRevision.current += 1;
              setDraft(e.target.value);
            }}
          />
          <div className="composer-toolbar">
            <span className="composer-trust"><OpenLockIcon size={13} /><span className="truncate">Alpha demo · plaintext messages</span></span>
            <span className="composer-hint"><kbd className="kbd">Enter</kbd> to send</span>
            <button
              type="submit"
              disabled={sending || !draft.trim()}
              className="send-button"
            >
              {sending ? '…' : <><SendIcon size={15} /><span className="sr-only">Send</span></>}
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}
