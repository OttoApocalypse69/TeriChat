import { useState } from 'react';
import { decodeOpaqueText } from '../lib/api';
import { avatarGradient } from '../lib/avatar';
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
import { PeopleIcon } from './icons';

interface Props {
  filter?: string;
  conversations: ChatConversation[];
  messagesByConversation: Map<string, ChatMessage[]>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onOpenDm: (peerHandle: string) => Promise<void>;
}

export default function ConversationList({
  filter = '',
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
  const ordered = sortConversations(conversations, messagesByConversation).filter(c =>
    `${conversationLabel(c)} ${conversationSublabel(c)}`.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase()),
  );

  return (
    <div className="conversation-list flex flex-col">
      <div className="nav-section-head">
        <h2 className="label-mono">Direct messages</h2>
      </div>
      <form onSubmit={open} className="nav-inline-form">
        <input
          className="field field-sm flex-1"
          aria-label="Peer handle"
          placeholder="peer handle → open DM"
          value={peer}
          onChange={(e) => setPeer(e.target.value)}
        />
        <button
          type="submit"
          disabled={busy || !peer.trim()}
          className="btn btn-secondary shrink-0"
        >
          {busy ? '…' : 'Open DM'}
        </button>
      </form>
      {error && <p className="text-alert px-1 pt-1">{error}</p>}
      <ul className="pt-1">
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
                aria-current={c.id === selectedId ? 'page' : undefined}
                className="conversation-row"
              >
                {c.kind === 'group'
                  ? <span aria-hidden className="tile h-8 w-8"><PeopleIcon /></span>
                  : <span aria-hidden className="avatar avatar-32" style={{ background: avatarGradient(c.peer_handle ?? c.id) }}>
                    {avatarInitial(c)}
                  </span>}
                <span className="conversation-row-body">
                  <span className="conversation-row-top">
                    <span className="conversation-row-name">{conversationLabel(c)}</span>
                    {when && <span className="conversation-row-time">{when}</span>}
                  </span>
                  <span className="conversation-row-preview">
                    {sub && <span className="conversation-row-sub">{sub} · </span>}
                    {preview}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
        {ordered.length === 0 && (
          <li className="navigation-no-results">
            {filter.trim() ? 'No matching conversations.' : 'No conversations yet — open a DM above.'}
          </li>
        )}
      </ul>
    </div>
  );
}
