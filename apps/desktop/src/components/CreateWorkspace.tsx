import { useEffect, useRef, useState } from 'react';
import { friendlyChannelError } from '../lib/workspaces';

export default function CreateWorkspace({ onCreate }: {
  onCreate: (name: string) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef(false);
  const live = useRef(false);
  useEffect(() => {
    live.current = true;
    return () => { live.current = false; };
  }, []);

  async function create(event: React.FormEvent) {
    event.preventDefault();
    if (pending.current || !name.trim()) return;
    pending.current = true;
    setBusy(true);
    setError(null);
    try {
      await onCreate(name.trim());
      if (live.current) setName('');
    } catch (err) {
      if (live.current) setError(friendlyChannelError(err));
    } finally {
      pending.current = false;
      if (live.current) setBusy(false);
    }
  }

  return (
    <div className="border-b border-zinc-800 p-2">
      <form onSubmit={create} className="flex gap-1" aria-label="Create workspace">
        <input
          aria-label="Workspace name"
          placeholder="new workspace name"
          className="min-w-0 flex-1 rounded bg-zinc-800 px-2 py-1.5 text-sm"
          value={name}
          disabled={busy}
          onChange={event => setName(event.target.value)}
        />
        <button type="submit" disabled={busy || !name.trim()}
          className="rounded bg-zinc-700 px-2 py-1 text-xs font-semibold disabled:opacity-40">
          {busy ? 'Creating…' : 'Create'}
        </button>
      </form>
      {error && <p role="alert" className="mt-1 text-xs text-red-400">{error}</p>}
    </div>
  );
}
