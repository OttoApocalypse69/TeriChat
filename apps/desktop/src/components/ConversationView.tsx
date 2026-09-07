import { useEffect, useState } from 'react';
import { decodeOpaqueText } from '../lib/api';
import type { ChatConversation, ChatMessage } from '../lib/store';

interface Props {
  conversation: ChatConversation | null;
  messages: ChatMessage[];
  meId: string;
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
  loading,
  sending,
  error,
  onSend,
  title,
}: Props) {
  const [draft, setDraft] = useState('');
  const [sendError, setSendError] = useState<string | null>(null);

  // Switching conversations (DM <-> DM, DM <-> channel, channel <-> channel)
  // must never leak the previous composer draft into the new conversation.
  const conversationId = conversation?.id ?? null;
  useEffect(() => {
    setDraft('');
    setSendError(null);
  }, [conversationId]);

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

  return (
    <div className="flex h-full flex-col">
      <div className="border-b border-zinc-800 px-3 py-2 text-xs text-zinc-400">
        {title && <span className="mr-2 font-semibold text-zinc-200">{title}</span>}
        <span className="font-mono">{conversation.id}</span>
        <span className="ml-2">{conversation.kind}</span>
      </div>
      <div className="flex-1 space-y-1 overflow-y-auto p-3">
        {loading && <p className="text-xs text-zinc-500">Loading history…</p>}
        {messages.map((m) => {
          const mine = m.sender_id === meId;
          return (
            <div
              key={m.id}
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
                #{m.seq} {mine ? 'you' : m.sender_id.slice(0, 8)}
              </p>
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
