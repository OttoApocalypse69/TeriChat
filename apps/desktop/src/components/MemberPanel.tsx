import { useCallback, useEffect, useRef, useState } from 'react';
import { ApiClient, type WorkspaceMemberBody } from '../lib/api';
import { avatarGradient } from '../lib/avatar';
import {
  friendlyMemberError, gateBan, gateGrant, gateKick, gateSetRole,
  grantableRoles, normalizeRole, roleHas, roleOrGuest, ROLE_NAMES, type RoleName,
} from '../lib/members';

// Permission roles reuse the community tag palette; the word always shows.
const ROLE_TAG: Record<RoleName, string> = {
  owner: 'tag-uv', admin: 'tag-sky', moderator: 'tag-amber', member: '', guest: 'tag-line',
};

interface Props {
  api: ApiClient;
  workspaceId: string;
  myRole: string;
  meId: string;
  onLeft: (workspaceId: string) => void;
  onMyRole?: (workspaceId: string, role: string) => void;
}

// Identity changes discard directory state before rendering another workspace.
export default function MemberPanel(props: Props) {
  return <MemberDirectoryPanel key={`${props.workspaceId}:${props.meId}`} {...props} />;
}

function MemberDirectoryPanel({ api, workspaceId, myRole, meId, onLeft, onMyRole }: Props) {
  const [members, setMembers] = useState<WorkspaceMemberBody[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [addHandle, setAddHandle] = useState('');
  const [addRole, setAddRole] = useState<RoleName>('member');
  const [unbanId, setUnbanId] = useState('');
  const [drafts, setDrafts] = useState<Record<string, RoleName>>({});
  const epoch = useRef(0);
  const mutation = useRef(false);
  const fetching = useRef(false);
  const failedCursor = useRef<string | undefined>(undefined);
  const actor = roleOrGuest(members.find(member => member.user_id === meId)?.role ?? myRole);
  const grantable = grantableRoles(actor);
  const busy = busyKey !== null || loading;

  const load = useCallback(async (after?: string) => {
    if (fetching.current) return;
    const current = epoch.current;
    fetching.current = true;
    failedCursor.current = after;
    setLoading(true);
    setLoadError(null);
    if (after === undefined) {
      setMembers([]);
      setNextCursor(null);
      setDrafts({});
    }
    try {
      const page = await api.listMembers(workspaceId, after);
      if (epoch.current !== current) return;
      if (page.next_cursor !== null && (page.members.length === 0 || page.next_cursor === after)) {
        throw new Error('Member page did not advance. Refresh the directory.');
      }
      setMembers(previous => [...new Map(
        [...(after === undefined ? [] : previous), ...page.members]
          .map(member => [member.user_id, member]),
      ).values()]);
      setNextCursor(page.next_cursor);
      const self = page.members.find(member => member.user_id === meId);
      if (self) onMyRole?.(workspaceId, self.role);
    } catch (err) {
      if (epoch.current === current) setLoadError(friendlyMemberError(err));
    } finally {
      if (epoch.current === current) {
        fetching.current = false;
        setLoading(false);
      }
    }
  }, [api, workspaceId, meId, onMyRole]);

  useEffect(() => {
    epoch.current += 1;
    fetching.current = false;
    mutation.current = false;
    setBusyKey(null);
    setError(null);
    void load();
    return () => { epoch.current += 1; };
  }, [load]);

  async function run(key: string, action: () => Promise<void>, success?: () => void) {
    if (mutation.current || fetching.current) return;
    mutation.current = true;
    const current = epoch.current;
    setBusyKey(key);
    setError(null);
    try {
      await action();
      if (epoch.current !== current) return;
      success?.();
      // Mutations invalidate every loaded page; obtain fresh roles and seats.
      if (key !== 'leave') await load();
    } catch (err) {
      if (epoch.current === current) setError(friendlyMemberError(err));
    } finally {
      if (epoch.current === current) {
        mutation.current = false;
        setBusyKey(null);
      }
    }
  }

  const buttonClass = 'btn btn-sm btn-secondary';
  return (
    <section aria-label="Workspace members" className="panel-section">
      <div className="panel-section-head">
        <h2 className="label-mono">
          Members · you are {actor}
        </h2>
        <button type="button" disabled={busy} onClick={() => void load()}
          className="btn btn-sm btn-ghost">Refresh members</button>
      </div>
      {error && <p role="alert" className="text-alert">{error}</p>}
      {loadError && <div role="alert" className="text-alert flex flex-col items-start gap-2">
        <p>{loadError}</p>
        <button type="button" disabled={busy} className={buttonClass}
          onClick={() => void load(failedCursor.current)}>Retry members</button>
      </div>}
      {loading && <p role="status" className="text-muted">Loading members…</p>}
      {!loading && !loadError && members.length === 0 &&
        <p className="text-muted">No current members.</p>}
      {grantable.length > 0 && (
        <form aria-label="Add member" className="flex gap-1" onSubmit={event => {
          event.preventDefault();
          const handle = addHandle.trim();
          if (!handle) return;
          const gate = gateGrant(actor, addRole);
          if (!gate.ok) { setError(gate.reason); return; }
          void run('add', () => api.addMember(workspaceId, { user_handle: handle, role: addRole }),
            () => setAddHandle(''));
        }}>
          <input aria-label="Member handle" placeholder="handle → add" value={addHandle}
            disabled={busy} onChange={event => setAddHandle(event.target.value)}
            className="field field-sm flex-1" />
          <select aria-label="New member role" value={addRole} disabled={busy}
            onChange={event => setAddRole(event.target.value as RoleName)}
            className="field field-sm w-auto">
            {grantable.map(role => <option key={role}>{role}</option>)}
          </select>
          <button type="submit" disabled={busy || !addHandle.trim()} className="btn btn-secondary shrink-0">
            {busyKey === 'add' ? 'Adding…' : 'Add'}
          </button>
        </form>
      )}
      <ul aria-label="Current members" className="flex flex-col gap-1.5">
        {members.map(member => {
          const uid = member.user_id;
          const self = uid === meId;
          const role = normalizeRole(member.role);
          const next = drafts[uid] ?? role ?? 'guest';
          const setGate = gateSetRole(actor, role, next);
          const kickGate = gateKick(actor, role);
          const banGate = gateBan(actor, role);
          return <li key={uid} data-member-id={uid} className="panel-row">
            <div className="flex items-center gap-2">
              <span aria-hidden className="avatar avatar-24" style={{ background: avatarGradient(member.handle) }}>
                {(member.display_name || member.handle).slice(0, 1).toUpperCase()}
              </span>
              <span className="min-w-0 flex-1 truncate text-[13px] font-semibold" title={uid}>
                {member.display_name || member.handle} <span className="font-normal text-ink-3">@{member.handle}</span>
                {self && <span className="meta-mono ml-1.5">you</span>}
              </span>
              <span className={`tag ${ROLE_TAG[role ?? 'guest']}`}>
                {member.role}
              </span>
            </div>
            {!self && <div className="mt-2 flex flex-wrap items-center gap-1">
              <select aria-label={`Role for ${member.handle}`} value={next}
                disabled={busy || !gateSetRole(actor, role, role ?? 'guest').ok}
                className="field field-sm w-auto"
                onChange={event => setDrafts(previous => ({ ...previous, [uid]: event.target.value as RoleName }))}>
                {ROLE_NAMES.map(option => <option key={option}>{option}</option>)}
              </select>
              <button type="button" disabled={busy || !setGate.ok} title={setGate.reason}
                onClick={() => void run(`role:${uid}`, () => api.setMemberRole(workspaceId, uid, next))}
                className={buttonClass}>Set role</button>
              <button type="button" disabled={busy || !kickGate.ok} title={kickGate.reason}
                onClick={() => void run(`kick:${uid}`, () => api.kickMember(workspaceId, uid))}
                className={buttonClass}>Kick</button>
              <button type="button" disabled={busy || !banGate.ok} title={banGate.reason}
                onClick={() => void run(`ban:${uid}`, () => api.banMember(workspaceId, uid), () => setUnbanId(uid))}
                className="btn btn-sm btn-danger">Ban</button>
            </div>}
          </li>;
        })}
      </ul>
      {nextCursor !== null && <button type="button" disabled={busy || loadError !== null}
        className={`self-start ${buttonClass}`} onClick={() => void load(nextCursor)}>Load more members</button>}
      <p className="meta-mono">
        {members.length} loaded{nextCursor !== null ? ' · more available' : ''}. Refresh to check for changes.
      </p>
      {roleHas(actor, 'BanMembers') && <form aria-label="Unban member" className="flex gap-1"
        onSubmit={event => {
          event.preventDefault();
          const uid = unbanId.trim();
          if (uid) void run('unban', () => api.unbanMember(workspaceId, uid), () => setUnbanId(''));
        }}>
        <input aria-label="User ID to unban" placeholder="user id → unban" value={unbanId} disabled={busy}
          onChange={event => setUnbanId(event.target.value)}
          className="field field-sm field-mono flex-1" />
        <button type="submit" disabled={busy || !unbanId.trim()} className="btn btn-secondary shrink-0">Unban</button>
      </form>}
      <button type="button" disabled={busy}
        onClick={() => void run('leave', () => api.leaveWorkspace(workspaceId), () => onLeft(workspaceId))}
        className="btn btn-danger btn-block"
        title="Leave this workspace (last owner cannot leave)">
        {busyKey === 'leave' ? 'Leaving…' : 'Leave workspace'}
      </button>
    </section>
  );
}
