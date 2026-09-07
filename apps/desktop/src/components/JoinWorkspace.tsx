import { useState } from 'react';
import { friendlyMemberError, reduceJoinState } from '../lib/members';

interface Props {
  /** Redeem the code; resolves with the joined workspace id to land in. */
  onJoin: (code: string) => Promise<string>;
}

/** Paste-an-invite-code join flow: idle -> joining -> joined/error. */
export default function JoinWorkspace({ onJoin }: Props) {
  const [code, setCode] = useState('');
  const [state, setState] = useState<
    ReturnType<typeof reduceJoinState>
  >({ phase: 'idle' });

  async function submit(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    const next = reduceJoinState(state, { type: 'submit', code });
    setState(next);
    if (next.phase !== 'joining') return;
    try {
      const workspaceId = await onJoin(next.code);
      setState(reduceJoinState(next, { type: 'ok', workspaceId }));
      setCode('');
    } catch (err) {
      setState(
        reduceJoinState(next, {
          type: 'fail',
          message: friendlyMemberError(err),
        }),
      );
    }
  }

  function reset(): void {
    setState({ phase: 'idle' });
    setCode('');
  }

  const joining = state.phase === 'joining';

  return (
    <div className="p-2">
      <h2 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">
        Join with code
      </h2>
      {state.phase === 'joined' ? (
        <div className="space-y-1">
          <p className="text-xs text-emerald-400">
            Joined workspace — you are in.
          </p>
          <button
            type="button"
            onClick={reset}
            className="rounded bg-zinc-800 px-2 py-1 text-xs"
          >
            Join another
          </button>
        </div>
      ) : (
        <form onSubmit={submit} className="space-y-1">
          <input
            className="w-full rounded bg-zinc-800 px-2 py-1.5 font-mono text-sm"
            placeholder="paste invite code"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
          <button
            type="submit"
            disabled={joining || !code.trim()}
            className="w-full rounded bg-zinc-700 py-1 text-xs font-semibold disabled:opacity-40"
          >
            {joining ? 'Joining…' : 'Join workspace'}
          </button>
          {state.phase === 'error' && (
            <p className="text-xs text-red-400">{state.message}</p>
          )}
        </form>
      )}
    </div>
  );
}
