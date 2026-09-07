import { useEffect, useRef, useState } from 'react';
import { ApiClient } from '../lib/api';
import {
  MemberDirectory,
  friendlyMemberError,
  gateBan,
  gateGrant,
  gateKick,
  gateSetRole,
  grantableRoles,
  roleHas,
  roleOrGuest,
  shortId,
  ROLE_NAMES,
  type RoleName,
} from '../lib/members';

interface Props {
  api: ApiClient;
  workspaceId: string;
  myRole: string;
  meId: string;
  myHandle: string;
  onLeft: (workspaceId: string) => void;
}

/**
 * Member list with roles plus manager actions. Rank-forbidden actions are
 * disabled up front (peers untouchable); server 403s still surface friendly.
 * The server has no list-members endpoint, so seats come from self + session
 * adds + the manager-visible audit log (see MemberDirectory).
 */
export default function MemberPanel({
  api,
  workspaceId,
  myRole,
  meId,
  myHandle,
  onLeft,
}: Props) {
  const actor = roleOrGuest(myRole);
  const dirRef = useRef<MemberDirectory | null>(null);
  if (dirRef.current === null) dirRef.current = new MemberDirectory();
  const dir = dirRef.current;
  const [, setVersion] = useState(0);
  const bump = () => setVersion((v) => v + 1);

  const [addHandle, setAddHandle] = useState('');
  const [addRole, setAddRole] = useState<RoleName>('member');
  const [unbanId, setUnbanId] = useState('');
  const [drafts, setDrafts] = useState<Record<string, RoleName>>({});
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const grantable = grantableRoles(actor);
  const canUnban = roleHas(actor, 'BanMembers');

  useEffect(() => {
    dir.seedSelf(meId, myHandle, actor);
    bump();
    let live = true;
    // Best-effort enrichment: non-managers meet the audit wall (403) and
    // simply keep self + session seats.
    void api
      .listAudit(workspaceId, 100)
      .then((rows) => {
        if (!live) return;
        dir.mergeAudit(rows);
        bump();
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, workspaceId]);

  async function run(key: string, fn: () => Promise<void>): Promise<void> {
    setBusyKey(key);
    setError(null);
    try {
      await fn();
      bump();
    } catch (err) {
      setError(friendlyMemberError(err));
    } finally {
      setBusyKey(null);
    }
  }

  async function add(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    const handle = addHandle.trim();
    if (!handle) return;
    const gate = gateGrant(actor, addRole);
    if (!gate.ok) {
      setError(gate.reason);
      return;
    }
    await run('add', async () => {
      await api.addMember(workspaceId, { user_handle: handle, role: addRole });
      dir.noteAdded(handle, addRole);
      setAddHandle('');
    });
  }

  const rows = dir.list();

  return (
    <div className="border-b border-zinc-800 p-2">
      <h2 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-zinc-400">
        Members · you are {actor}
      </h2>
      {error && <p className="mb-1 text-xs text-red-400">{error}</p>}

      {grantable.length > 0 && (
        <form onSubmit={add} className="mb-2 flex gap-1">
          <input
            className="min-w-0 flex-1 rounded bg-zinc-800 px-2 py-1.5 text-sm"
            placeholder="handle → add"
            value={addHandle}
            onChange={(e) => setAddHandle(e.target.value)}
          />
          <select
            className="shrink-0 rounded bg-zinc-800 px-1 py-1.5 text-sm"
            value={addRole}
            onChange={(e) => setAddRole(e.target.value as RoleName)}
            title="Role for the new member (never owner)"
          >
            {grantable.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
          <button
            type="submit"
            disabled={busyKey === 'add' || !addHandle.trim()}
            className="shrink-0 rounded bg-zinc-700 px-2 py-1 text-xs font-semibold disabled:opacity-40"
          >
            {busyKey === 'add' ? '…' : 'Add'}
          </button>
        </form>
      )}

      <ul className="space-y-1.5">
        {rows.map((m) => {
          const isSelf = m.userId === meId;
          // Local const so narrowing survives the gate calls below.
          const uid: string | null = isSelf ? null : m.userId;
          const label = m.handle ?? (m.userId ? shortId(m.userId) : m.key);
          const setGate =
            uid !== null
              ? gateSetRole(actor, m.role, drafts[uid] ?? m.role ?? 'member')
              : { ok: false, reason: '' };
          const kickGate =
            uid !== null ? gateKick(actor, m.role) : { ok: false, reason: '' };
          const banGate =
            uid !== null ? gateBan(actor, m.role) : { ok: false, reason: '' };
          return (
            <li key={m.key} className="rounded bg-zinc-900 p-1.5">
              <div className="flex items-center gap-1.5">
                <span className="min-w-0 flex-1 truncate text-sm">
                  {label}
                  {isSelf && (
                    <span className="ml-1 text-[10px] uppercase text-zinc-500">
                      you
                    </span>
                  )}
                </span>
                <span className="shrink-0 rounded bg-zinc-800 px-1.5 py-0.5 text-[10px] uppercase text-zinc-400">
                  {m.role ?? 'unknown'}
                </span>
                {m.banned && (
                  <span className="shrink-0 rounded bg-red-900 px-1.5 py-0.5 text-[10px] uppercase text-red-200">
                    banned
                  </span>
                )}
              </div>
              {!isSelf && uid !== null && !m.banned && (
                <div className="mt-1 flex flex-wrap items-center gap-1">
                  <select
                    className="rounded bg-zinc-800 px-1 py-1 text-xs"
                    value={drafts[uid] ?? m.role ?? 'member'}
                    onChange={(e) =>
                      setDrafts((d) => ({
                        ...d,
                        [uid]: e.target.value as RoleName,
                      }))
                    }
                    disabled={!setGate.ok}
                    title={setGate.ok ? 'New role' : setGate.reason}
                  >
                    {ROLE_NAMES.map((r) => (
                      <option key={r} value={r}>
                        {r}
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    disabled={!setGate.ok || busyKey === `role:${uid}`}
                    title={setGate.ok ? 'Apply role' : setGate.reason}
                    onClick={() => {
                      const next = drafts[uid] ?? m.role ?? 'member';
                      const target = uid;
                      void run(`role:${target}`, async () => {
                        await api.setMemberRole(workspaceId, target, next);
                        dir.applyRole(target, next);
                      });
                    }}
                    className="rounded bg-zinc-800 px-1.5 py-1 text-[11px] disabled:opacity-40"
                  >
                    {busyKey === `role:${uid}` ? '…' : 'Set role'}
                  </button>
                  <button
                    type="button"
                    disabled={!kickGate.ok || busyKey === `kick:${uid}`}
                    title={kickGate.ok ? 'Kick from workspace' : kickGate.reason}
                    onClick={() => {
                      const target = uid;
                      void run(`kick:${target}`, async () => {
                        await api.kickMember(workspaceId, target);
                        dir.removeById(target);
                      });
                    }}
                    className="rounded bg-zinc-800 px-1.5 py-1 text-[11px] disabled:opacity-40"
                  >
                    {busyKey === `kick:${uid}` ? '…' : 'Kick'}
                  </button>
                  <button
                    type="button"
                    disabled={!banGate.ok || busyKey === `ban:${uid}`}
                    title={
                      banGate.ok
                        ? 'Ban from workspace (empty reason)'
                        : banGate.reason
                    }
                    onClick={() => {
                      const target = uid;
                      void run(`ban:${target}`, async () => {
                        await api.banMember(workspaceId, target);
                        dir.removeById(target);
                        dir.markBanned(target, true);
                      });
                    }}
                    className="rounded bg-zinc-800 px-1.5 py-1 text-[11px] text-red-300 disabled:opacity-40"
                  >
                    {busyKey === `ban:${uid}` ? '…' : 'Ban'}
                  </button>
                </div>
              )}
              {!isSelf && uid !== null && m.banned && (
                <div className="mt-1">
                  <button
                    type="button"
                    disabled={!canUnban || busyKey === `unban:${uid}`}
                    title={
                      canUnban ? 'Lift the ban' : 'unbanning requires admin or owner'
                    }
                    onClick={() => {
                      const target = uid;
                      void run(`unban:${target}`, async () => {
                        await api.unbanMember(workspaceId, target);
                        dir.markBanned(target, false);
                      });
                    }}
                    className="rounded bg-zinc-800 px-1.5 py-1 text-[11px] disabled:opacity-40"
                  >
                    {busyKey === `unban:${uid}` ? '…' : 'Unban'}
                  </button>
                </div>
              )}
              {!isSelf && !m.userId && (
                <p className="mt-1 text-[11px] text-zinc-500">
                  Just added — id unknown until the server lists the seat.
                </p>
              )}
            </li>
          );
        })}
      </ul>

      {canUnban && (
        <form
          className="mt-2 flex gap-1"
          onSubmit={(e) => {
            e.preventDefault();
            const id = unbanId.trim();
            if (!id) return;
            void run('unban-form', async () => {
              await api.unbanMember(workspaceId, id);
              dir.markBanned(id, false);
              setUnbanId('');
            });
          }}
        >
          <input
            className="min-w-0 flex-1 rounded bg-zinc-800 px-2 py-1.5 font-mono text-xs"
            placeholder="user id → unban"
            value={unbanId}
            onChange={(e) => setUnbanId(e.target.value)}
          />
          <button
            type="submit"
            disabled={busyKey === 'unban-form' || !unbanId.trim()}
            className="shrink-0 rounded bg-zinc-700 px-2 py-1 text-xs font-semibold disabled:opacity-40"
          >
            {busyKey === 'unban-form' ? '…' : 'Unban'}
          </button>
        </form>
      )}

      <button
        type="button"
        disabled={busyKey === 'leave'}
        onClick={() =>
          void run('leave', async () => {
            await api.leaveWorkspace(workspaceId);
            onLeft(workspaceId);
          })
        }
        className="mt-2 w-full rounded bg-zinc-800 py-1 text-xs text-red-300 disabled:opacity-40"
        title="Leave this workspace (last owner cannot leave)"
      >
        {busyKey === 'leave' ? 'Leaving…' : 'Leave workspace'}
      </button>
    </div>
  );
}
