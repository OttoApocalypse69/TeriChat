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
    <div className="flex flex-col gap-1.5">
      <h2 className="label-mono">
        Join with code
      </h2>
      {state.phase === 'joined' ? (
        <div className="flex flex-col items-start gap-1.5">
          <p className="text-success">
            Joined workspace — you are in.
          </p>
          <button
            type="button"
            onClick={reset}
            className="btn btn-sm btn-secondary"
          >
            Join another
          </button>
        </div>
      ) : (
        <form onSubmit={submit} className="flex flex-col gap-1">
          <input
            className="field field-sm field-mono"
            placeholder="paste invite code"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
          <button
            type="submit"
            disabled={joining || !code.trim()}
            className="btn btn-secondary btn-block"
          >
            {joining ? 'Joining…' : 'Join workspace'}
          </button>
          {state.phase === 'error' && (
            <p className="text-alert">{state.message}</p>
          )}
        </form>
      )}
    </div>
  );
}
