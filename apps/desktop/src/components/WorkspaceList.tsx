import type { WorkspaceBody } from '../lib/api';

interface Props {
  workspaces: WorkspaceBody[];
  selectedWorkspaceId: string | null;
  loading: boolean;
  error: string | null;
  onSelect: (id: string) => void;
  onRetry: () => void;
}

export default function WorkspaceList({
  workspaces,
  selectedWorkspaceId,
  loading,
  error,
  onSelect,
  onRetry,
}: Props) {
  return (
    <div className="border-b border-zinc-800 p-2">
      <div className="mb-1 flex items-center justify-between">
        <h2 className="text-[11px] font-semibold uppercase tracking-wide text-zinc-400">
          Workspaces
        </h2>
        {loading && <span className="text-[11px] text-zinc-500">…</span>}
      </div>
      {error && (
        <div className="mb-1">
          <p className="text-xs text-red-400">{error}</p>
          <button
            type="button"
            onClick={onRetry}
            className="mt-1 rounded bg-zinc-800 px-2 py-0.5 text-xs"
          >
            Retry
          </button>
        </div>
      )}
      {!loading && !error && workspaces.length === 0 && (
        <p className="px-1 py-1 text-xs text-zinc-500">
          No workspaces yet — ask for an invite.
        </p>
      )}
      <ul className="max-h-32 space-y-0.5 overflow-y-auto">
        {workspaces.map((w) => (
          <li key={w.id}>
            <button
              type="button"
              onClick={() => onSelect(w.id)}
              aria-pressed={w.id === selectedWorkspaceId}
              className={`w-full truncate rounded px-2 py-1.5 text-left text-sm ${
                w.id === selectedWorkspaceId
                  ? 'bg-zinc-700 font-semibold'
                  : 'hover:bg-zinc-800'
              }`}
              title={`${w.name} (${w.my_role})`}
            >
              {w.name}
              <span className="ml-1 text-[10px] uppercase text-zinc-500">
                {w.my_role}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
