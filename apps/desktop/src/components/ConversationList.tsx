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
import { PeopleIcon, PlusIcon } from './icons';

interface Props {
  filter?: string;
  meId?: string;
  conversations: ChatConversation[];
  messagesByConversation: Map<string, ChatMessage[]>;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onOpenDm: (peerHandle: string) => Promise<void>;
  /** Resolves once the group exists; rejects with a user-facing message. */
  onCreateGroup?: (memberInput: string) => Promise<void>;
}

export default function ConversationList({
  filter = '',
  meId,
  conversations,
  messagesByConversation,
  selectedId,
  onSelect,
  onOpenDm,
  onCreateGroup,
}: Props) {
  const [peer, setPeer] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [groupOpen, setGroupOpen] = useState(false);
  const [groupMembers, setGroupMembers] = useState('');
  const [groupError, setGroupError] = useState<string | null>(null);
  const [groupBusy, setGroupBusy] = useState(false);

  async function createGroup(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    if (!onCreateGroup || groupBusy || !groupMembers.trim()) return;
    setGroupError(null);
    setGroupBusy(true);
    try {
      await onCreateGroup(groupMembers);
      setGroupMembers('');
      setGroupOpen(false);
    } catch (err) {
      setGroupError(err instanceof Error ? err.message : 'group creation failed');
    } finally {
      setGroupBusy(false);
    }
  }

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
    `${conversationLabel(c, meId)} ${conversationSublabel(c)}`.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase()),
  );

  return (
    <div className="conversation-list flex flex-col">
      <div className="nav-section-head">
        <h2 className="label-mono">Direct messages</h2>
        {onCreateGroup && <button type="button" className="btn btn-sm btn-ghost" aria-expanded={groupOpen}
          aria-controls="new-group-form" onClick={() => { setGroupOpen(open => !open); setGroupError(null); }}>
          <PlusIcon size={13} />New group
        </button>}
      </div>
      {onCreateGroup && groupOpen && (
        <form id="new-group-form" aria-label="New group" onSubmit={createGroup} className="flex flex-col gap-1 pb-1">
          <input
            className="field field-sm"
            aria-label="Group member handles"
            placeholder="handles, comma-separated"
            value={groupMembers}
            disabled={groupBusy}
            onChange={(e) => setGroupMembers(e.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
          <button type="submit" disabled={groupBusy || !groupMembers.trim()} className="btn btn-primary btn-block">
            {groupBusy ? 'Creating…' : 'Create group'}
          </button>
          {groupError && <p role="alert" className="text-alert">{groupError}</p>}
        </form>
      )}
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
                    <span className="conversation-row-name">{conversationLabel(c, meId)}</span>
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
