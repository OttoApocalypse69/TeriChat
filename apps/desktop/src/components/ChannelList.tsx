import { useState } from 'react';
import { friendlyChannelError } from '../lib/workspaces';
import type { ChannelBody } from '../lib/api';

interface Props {
  filter?: string;
  /** Private unread counts keyed by channel conversation id. */
  unreadByConversation?: Map<string, number>;
  workspaceName: string | null;
  channels: ChannelBody[];
  selectedChannelId: string | null;
  loading: boolean;
  error: string | null;
  onSelect: (id: string) => void;
  onCreateChannel: (name: string) => Promise<void>;
}

export default function ChannelList({
  filter = '',
  unreadByConversation,
  workspaceName,
  channels,
  selectedChannelId,
  loading,
  error,
  onSelect,
  onCreateChannel,
}: Props) {
  const [name, setName] = useState('');
  const [createError, setCreateError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function create(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    if (!name.trim()) return;
    setCreateError(null);
    setBusy(true);
    try {
      await onCreateChannel(name.trim());
      setName('');
    } catch (err) {
      // 403 (missing MANAGE_CHANNELS) lands here as a plain message.
      setCreateError(friendlyChannelError(err));
    } finally {
      setBusy(false);
    }
  }

  if (!workspaceName) {
    return (
      <div className="channel-list">
        <div className="nav-section-head">
          <h2 className="label-mono">Channels</h2>
        </div>
        <p className="navigation-no-results">
          Select a workspace to see channels.
        </p>
      </div>
    );
  }

  const query = filter.trim().toLocaleLowerCase().replace(/^#/, '');
  const visible = channels.filter(c => c.name.toLocaleLowerCase().includes(query));
  return (
    <div className="channel-list">
      <div className="nav-section-head">
        {/* The workspace name is the navigator title; keep it for assistive tech. */}
        <h2 className="label-mono">
          Channels<span className="sr-only"> · {workspaceName}</span>
        </h2>
        {loading && <span className="meta-mono" aria-hidden>…</span>}
      </div>
      {error && <p className="text-alert px-2 pb-1">{error}</p>}
      <ul>
        {visible.map((c) => {
          const unread = unreadByConversation?.get(c.conversation_id) ?? 0;
          return (
            <li key={c.id}>
              <button
                type="button"
                onClick={() => onSelect(c.id)}
                aria-current={c.id === selectedChannelId ? 'page' : undefined}
                aria-describedby={unread > 0 ? `unread-${c.conversation_id}` : undefined}
                className={`nav-row${unread > 0 ? ' nav-row--unread' : ''}`}
                title={`#${c.name}`}
              >
                {/* One inline run keeps the accessible name "#name". */}
                <span className="nav-row-label"><span className="channel-glyph">#</span>{c.name}</span>
                {unread > 0 && <span id={`unread-${c.conversation_id}`} hidden>{unread} unread</span>}
              </button>
            </li>
          );
        })}
      </ul>
      {filter.trim() && channels.length > 0 && visible.length === 0 && (
        <p className="navigation-no-results">No matching channels.</p>
      )}
      {!loading && !error && channels.length === 0 && (
        <p className="navigation-no-results">
          No channels yet — create one below.
        </p>
      )}
      <form onSubmit={create} className="nav-inline-form">
        <input
          className="field field-sm flex-1"
          aria-label="New channel name"
          placeholder="new channel name"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <button
          type="submit"
          disabled={busy || !name.trim()}
          className="btn btn-secondary shrink-0"
        >
          {busy ? '…' : 'Add'}
        </button>
      </form>
      {createError && <p className="text-alert px-1 pt-1">{createError}</p>}
    </div>
  );
}
