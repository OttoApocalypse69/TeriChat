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
    <div>
      <form onSubmit={create} className="flex gap-1" aria-label="Create workspace">
        <input
          aria-label="Workspace name"
          placeholder="new workspace name"
          className="field field-sm flex-1"
          value={name}
          disabled={busy}
          onChange={event => setName(event.target.value)}
        />
        <button type="submit" disabled={busy || !name.trim()}
          className="btn btn-primary shrink-0">
          {busy ? 'Creating…' : 'Create'}
        </button>
      </form>
      {error && <p role="alert" className="text-alert mt-1">{error}</p>}
    </div>
  );
}
