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
    <div className="rail-workspaces">
      <div className="mb-1 flex items-center justify-between">
        <h2 className="sr-only">
          Workspaces
        </h2>
        {loading && <span className="text-[11px] text-zinc-500">…</span>}
      </div>
      {error && (
        <div className="mb-1">
          <p className="rail-error" title={error}>!</p>
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
        <p className="rail-empty" title="No workspaces yet — create one in the sidebar or join with an invite.">
          —
        </p>
      )}
      <ul>
        {workspaces.map((w) => (
          <li key={w.id}>
            <button
              type="button"
              onClick={() => onSelect(w.id)}
              aria-pressed={w.id === selectedWorkspaceId}
              aria-label={`${w.name} ${w.my_role}`}
              className="rail-workspace"
              title={`${w.name} (${w.my_role})`}
            >
              <span aria-hidden>{w.name.trim().slice(0, 2).toUpperCase()}</span>
              <span className="sr-only">{w.name} {w.my_role}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
