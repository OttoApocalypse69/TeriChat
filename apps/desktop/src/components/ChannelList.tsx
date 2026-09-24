import { useState } from 'react';
import { friendlyChannelError } from '../lib/workspaces';
import type { ChannelBody } from '../lib/api';

interface Props {
  filter?: string;
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
      <div className="border-b border-zinc-800 p-2">
        <h2 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">
          Channels
        </h2>
        <p className="px-1 py-1 text-xs text-zinc-500">
          Select a workspace to see channels.
        </p>
      </div>
    );
  }

  return (
    <div className="channel-list border-b border-zinc-800 p-2">
      <div className="mb-1 flex items-center justify-between">
        <h2 className="text-[11px] font-semibold uppercase tracking-wide text-zinc-400">
          Channels · {workspaceName}
        </h2>
        {loading && <span className="text-[11px] text-zinc-500">…</span>}
      </div>
      {error && <p className="mb-1 text-xs text-red-400">{error}</p>}
      <ul className="max-h-40 space-y-0.5 overflow-y-auto">
        {channels.filter(c => c.name.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase().replace(/^#/, ''))).map((c) => (
          <li key={c.id}>
            <button
              type="button"
              onClick={() => onSelect(c.id)}
              aria-current={c.id === selectedChannelId ? 'page' : undefined}
              className={`w-full truncate rounded px-2 py-1.5 text-left text-sm ${
                c.id === selectedChannelId
                  ? 'bg-emerald-950 font-semibold'
                  : 'hover:bg-zinc-800'
              }`}
              title={`#${c.name}`}
            >
              <span className="mr-1 text-zinc-500">#</span>
              {c.name}
            </button>
          </li>
        ))}
      </ul>
      {filter.trim() && channels.length > 0 && !channels.some(c => c.name.toLocaleLowerCase().includes(filter.trim().toLocaleLowerCase().replace(/^#/, ''))) && (
        <p className="navigation-no-results">No matching channels.</p>
      )}
      {!loading && !error && channels.length === 0 && (
        <p className="px-1 py-1 text-xs text-zinc-500">
          No channels yet — create one below.
        </p>
      )}
      <form onSubmit={create} className="mt-2 flex gap-1">
        <input
          className="min-w-0 flex-1 rounded bg-zinc-800 px-2 py-1.5 text-sm"
          aria-label="New channel name"
          placeholder="new channel name"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <button
          type="submit"
          disabled={busy || !name.trim()}
          className="shrink-0 rounded bg-zinc-700 px-2 py-1 text-xs font-semibold disabled:opacity-40"
        >
          {busy ? '…' : 'Add'}
        </button>
      </form>
      {createError && <p className="mt-1 text-xs text-red-400">{createError}</p>}
    </div>
  );
}
