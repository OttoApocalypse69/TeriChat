import { useEffect, useState } from 'react';
import { ApiClient, type InviteBody } from '../lib/api';
import {
  canUseInvitePanel,
  friendlyMemberError,
  grantableRoles,
  roleOrGuest,
  type RoleName,
} from '../lib/members';

interface Props {
  api: ApiClient;
  workspaceId: string;
  myRole: string;
}

const TTL_OPTIONS: { label: string; secs: number | null }[] = [
  { label: 'Never expires', secs: null },
  { label: '1 hour', secs: 3600 },
  { label: '24 hours', secs: 86400 },
  { label: '7 days', secs: 604800 },
  { label: '30 days', secs: 2592000 },
];

async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Fall through to the legacy path.
  }
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    ta.remove();
    return ok;
  } catch {
    return false;
  }
}

function usesLabel(invite: InviteBody): string {
  return invite.max_uses === null
    ? `${invite.uses} uses`
    : `${invite.uses}/${invite.max_uses} uses`;
}

/** Manager-only invite panel: create (role/TTL/use-cap), list+copy, revoke. */
export default function InvitePanel({ api, workspaceId, myRole }: Props) {
  const actor = roleOrGuest(myRole);
  const [invites, setInvites] = useState<InviteBody[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [role, setRole] = useState<RoleName>('member');
  const [ttl, setTtl] = useState<number | null>(null);
  const [maxUses, setMaxUses] = useState('');
  const [creating, setCreating] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const grantable = grantableRoles(actor);

  useEffect(() => {
    if (!canUseInvitePanel(actor)) return;
    let live = true;
    setLoading(true);
    setError(null);
    void api
      .listInvites(workspaceId)
      .then((rows) => {
        if (!live) return;
        setInvites(
          [...rows].sort((a, b) => b.created_at.localeCompare(a.created_at)),
        );
      })
      .catch((err: unknown) => {
        if (live) setError(friendlyMemberError(err));
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [api, workspaceId]);

  // Keep the selected grant valid when the actor role changes.
  useEffect(() => {
    if (!grantable.includes(role)) {
      setRole(grantable[grantable.length - 1] ?? 'member');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId]);

  async function create(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    setError(null);
    let cap: number | undefined;
    if (maxUses.trim() !== '') {
      cap = Number(maxUses.trim());
      if (!Number.isInteger(cap) || cap <= 0) {
        setError('use cap must be a positive whole number (or blank)');
        return;
      }
    }
    setCreating(true);
    try {
      const invite = await api.createInvite(workspaceId, {
        initial_role: role,
        ...(ttl !== null ? { expires_in_secs: ttl } : {}),
        ...(cap !== undefined ? { max_uses: cap } : {}),
      });
      setInvites((prev) => [invite, ...prev]);
      setMaxUses('');
    } catch (err) {
      setError(friendlyMemberError(err));
    } finally {
      setCreating(false);
    }
  }

  async function revoke(inviteId: string): Promise<void> {
    setBusyId(inviteId);
    setError(null);
    try {
      await api.revokeInvite(workspaceId, inviteId);
      setInvites((prev) =>
        prev.map((inv) =>
          inv.id === inviteId ? { ...inv, revoked: true } : inv,
        ),
      );
    } catch (err) {
      setError(friendlyMemberError(err));
    } finally {
      setBusyId(null);
    }
  }

  async function copy(invite: InviteBody): Promise<void> {
    const ok = await copyText(invite.code);
    setCopiedId(ok ? invite.id : null);
    if (!ok) setError('copy failed — select the code and copy manually');
  }

  // Non-managers never see invite codes (render gate after all hooks).
  if (!canUseInvitePanel(actor)) return null;

  return (
    <div className="border-b border-zinc-800 p-2">
      <div className="mb-1 flex items-center justify-between">
        <h2 className="text-[11px] font-semibold uppercase tracking-wide text-zinc-400">
          Invites · {actor}
        </h2>
        {loading && <span className="text-[11px] text-zinc-500">…</span>}
      </div>
      {error && <p className="mb-1 text-xs text-red-400">{error}</p>}
      <form onSubmit={create} className="space-y-1">
        <div className="flex gap-1">
          <select
            className="min-w-0 flex-1 rounded bg-zinc-800 px-2 py-1.5 text-sm"
            value={role}
            onChange={(e) => setRole(e.target.value as RoleName)}
            title="Role granted on accept (never owner)"
          >
            {grantable.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
          <select
            className="min-w-0 flex-1 rounded bg-zinc-800 px-2 py-1.5 text-sm"
            value={ttl === null ? '' : String(ttl)}
            onChange={(e) =>
              setTtl(e.target.value === '' ? null : Number(e.target.value))
            }
            title="Invite lifetime"
          >
            {TTL_OPTIONS.map((o) => (
              <option key={o.label} value={o.secs === null ? '' : o.secs}>
                {o.label}
              </option>
            ))}
          </select>
        </div>
        <div className="flex gap-1">
          <input
            className="min-w-0 flex-1 rounded bg-zinc-800 px-2 py-1.5 text-sm"
            placeholder="use cap (blank = ∞)"
            value={maxUses}
            onChange={(e) => setMaxUses(e.target.value)}
            inputMode="numeric"
          />
          <button
            type="submit"
            disabled={creating}
            className="shrink-0 rounded bg-zinc-700 px-2 py-1 text-xs font-semibold disabled:opacity-40"
          >
            {creating ? '…' : 'Create'}
          </button>
        </div>
      </form>
      <ul className="mt-2 space-y-1.5">
        {invites.map((inv) => (
          <li
            key={inv.id}
            className={`rounded bg-zinc-900 p-1.5 ${inv.revoked ? 'opacity-50' : ''}`}
          >
            <div className="flex items-center gap-1">
              <code className="min-w-0 flex-1 truncate font-mono text-xs text-zinc-200">
                {inv.code}
              </code>
              <button
                type="button"
                onClick={() => void copy(inv)}
                disabled={inv.revoked}
                className="shrink-0 rounded bg-zinc-800 px-1.5 py-0.5 text-[11px] disabled:opacity-40"
                title="Copy invite code"
              >
                {copiedId === inv.id ? 'Copied' : 'Copy'}
              </button>
              <button
                type="button"
                onClick={() => void revoke(inv.id)}
                disabled={inv.revoked || busyId === inv.id}
                className="shrink-0 rounded bg-zinc-800 px-1.5 py-0.5 text-[11px] text-red-300 disabled:opacity-40"
                title="Revoke this invite code"
              >
                {inv.revoked
                  ? 'Revoked'
                  : busyId === inv.id
                    ? '…'
                    : 'Revoke'}
              </button>
            </div>
            <p className="mt-0.5 text-[11px] text-zinc-500">
              {inv.initial_role} · {usesLabel(inv)} ·{' '}
              {inv.expires_at
                ? `expires ${new Date(inv.expires_at).toLocaleString()}`
                : 'never expires'}
            </p>
          </li>
        ))}
      </ul>
      {!loading && invites.length === 0 && (
        <p className="px-1 py-1 text-xs text-zinc-500">
          No invites yet — create one above.
        </p>
      )}
    </div>
  );
}
