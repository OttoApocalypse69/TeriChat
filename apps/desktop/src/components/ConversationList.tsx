import { useState } from 'react';
import type { ChatConversation } from '../lib/store';

interface Props {
  conversations: ChatConversation[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onOpenDm: (peerHandle: string) => Promise<void>;
}

export default function ConversationList({
  conversations,
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
        {conversations.map((c) => (
          <li key={c.id}>
            <button
              type="button"
              onClick={() => onSelect(c.id)}
              className={`w-full rounded px-2 py-1.5 text-left text-sm ${
                c.id === selectedId
                  ? 'bg-zinc-700 font-semibold'
                  : 'hover:bg-zinc-800'
              }`}
            >
              <span className="mr-1 rounded bg-zinc-800 px-1 text-[10px] uppercase text-zinc-400">
                {c.kind}
              </span>
              <span className="font-mono text-xs">{c.id.slice(0, 8)}…</span>
              <span className="ml-1 text-[11px] text-zinc-500">
                {c.members.length > 0 ? `${c.members.length} members` : ''}
              </span>
            </button>
          </li>
        ))}
        {conversations.length === 0 && (
          <li className="px-2 py-4 text-xs text-zinc-500">
            No conversations yet — open a DM above.
          </li>
        )}
      </ul>
    </div>
  );
}
