import type { WorkspaceBody } from '../lib/api';

interface Props {
  workspaces: WorkspaceBody[];
  /** Workspaces with unread channels among those loaded this session. */
  unreadWorkspaceIds?: Set<string>;
  selectedWorkspaceId: string | null;
  loading: boolean;
  error: string | null;
  onSelect: (id: string) => void;
  onRetry: () => void;
}

export default function WorkspaceList({
  workspaces,
  unreadWorkspaceIds,
  selectedWorkspaceId,
  loading,
  error,
  onSelect,
  onRetry,
}: Props) {
  return (
    <div className="rail-workspaces">
      <h2 className="sr-only">
        Workspaces
      </h2>
      <span className="rail-label" aria-hidden>{loading ? '···' : 'SPACES'}</span>
      {error && (
        <div className="flex flex-col items-center gap-1">
          <p className="rail-error" title={error}>!</p>
          <button
            type="button"
            onClick={onRetry}
            className="btn btn-sm btn-secondary"
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
              aria-describedby={unreadWorkspaceIds?.has(w.id) ? `unread-ws-${w.id}` : undefined}
              className={`rail-tile rail-workspace${unreadWorkspaceIds?.has(w.id) ? ' rail-workspace--unread' : ''}`}
              title={`${w.name} (${w.my_role})`}
            >
              <span className="rail-tile-rim" aria-hidden />
              <span className="rail-tile-face" aria-hidden>{w.name.trim().slice(0, 2).toUpperCase()}</span>
              <span className="sr-only">{w.name} {w.my_role}</span>
              {unreadWorkspaceIds?.has(w.id) && <span id={`unread-ws-${w.id}`} hidden>Unread messages</span>}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
