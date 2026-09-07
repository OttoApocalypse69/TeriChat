import { useState } from 'react';
import { decodeOpaqueText } from '../lib/api';
import {
  avatarInitial,
  conversationLabel,
  conversationSublabel,
  formatListTime,
  lastActivityAt,
  sortConversations,
  truncatePreview,
  type ChatConversation,
  type ChatMessage,
} from '../lib/store';

interface Props {
  conversations: ChatConversation[];
  messagesByConversation: Map<string, ChatMessage[]>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onOpenDm: (peerHandle: string) => Promise<void>;
}

export default function ConversationList({
  conversations,
  messagesByConversation,
  selectedId,
  onSelect,
  onOpenDm,
}: Props) {
  const [peer, setPeer] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function open(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    if (!peer.trim()) return;
    setError(null);
    setBusy(true);
    try {
      await onOpenDm(peer.trim());
      setPeer('');
    } catch (err) {
      setError(err instanceof Error ? err.message : 'open DM failed');
    } finally {
      setBusy(false);
    }
  }

  // Newest activity first; quiet conversations fall back to the
  // server-reported last position so reloads keep a stable order.
  const ordered = sortConversations(conversations, messagesByConversation);

  return (
    <div className="flex h-full flex-col">
      <form onSubmit={open} className="space-y-1 border-b border-zinc-800 p-2">
        <input
          className="w-full rounded bg-zinc-800 px-2 py-1.5 text-sm"
          placeholder="peer handle → open DM"
          value={peer}
          onChange={(e) => setPeer(e.target.value)}
        />
        <button
          type="submit"
          disabled={busy || !peer.trim()}
          className="w-full rounded bg-zinc-700 py-1 text-xs font-semibold disabled:opacity-40"
        >
          {busy ? '…' : 'Open DM'}
        </button>
        {error && <p className="text-xs text-red-400">{error}</p>}
      </form>
      <ul className="flex-1 overflow-y-auto p-1">
        {ordered.map((c) => {
          const loaded = messagesByConversation.get(c.id) ?? [];
          const last = loaded.length > 0 ? loaded[loaded.length - 1] : null;
          const preview = last
            ? truncatePreview(decodeOpaqueText(last.ciphertext_b64))
            : c.last_seq != null
              ? `seq ${c.last_seq}`
              : 'No messages yet';
          const when = formatListTime(
            lastActivityAt(c, loaded.length > 0 ? loaded : undefined),
          );
          const sub = conversationSublabel(c);
          return (
            <li key={c.id}>
              <button
                type="button"
                onClick={() => onSelect(c.id)}
                className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${
                  c.id === selectedId
                    ? 'bg-zinc-700 font-semibold'
                    : 'hover:bg-zinc-800'
                }`}
              >
                <span
                  aria-hidden
                  className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-zinc-700 text-xs font-bold text-zinc-200"
                >
                  {avatarInitial(c)}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline justify-between gap-2">
                    <span className="truncate">{conversationLabel(c)}</span>
                    {when && (
                      <span className="shrink-0 text-[10px] font-normal text-zinc-500">
                        {when}
                      </span>
                    )}
                  </span>
                  <span className="block truncate text-xs font-normal text-zinc-500">
                    {sub ? `${sub} · ` : ''}
                    {preview}
                  </span>
                </span>
                <span className="shrink-0 rounded bg-zinc-800 px-1 text-[10px] uppercase text-zinc-400">
                  {c.kind}
                </span>
              </button>
            </li>
          );
        })}
        {ordered.length === 0 && (
          <li className="px-2 py-4 text-xs text-zinc-500">
            No conversations yet — open a DM above.
          </li>
        )}
      </ul>
    </div>
  );
}
