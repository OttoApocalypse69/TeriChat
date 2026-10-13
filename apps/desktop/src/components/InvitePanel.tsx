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
    <section aria-label="Workspace invites" className="panel-section">
      <div className="panel-section-head">
        <h2 className="label-mono">
          Invites · {actor}
        </h2>
        {loading && <span className="meta-mono" aria-hidden>…</span>}
      </div>
      {error && <p className="text-alert">{error}</p>}
      <form onSubmit={create} className="flex flex-col gap-1">
        <div className="flex gap-1">
          <select
            className="field field-sm flex-1"
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
            className="field field-sm flex-1"
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
            className="field field-sm flex-1"
            placeholder="use cap (blank = ∞)"
            value={maxUses}
            onChange={(e) => setMaxUses(e.target.value)}
            inputMode="numeric"
          />
          <button
            type="submit"
            disabled={creating}
            className="btn btn-primary shrink-0"
          >
            {creating ? '…' : 'Create'}
          </button>
        </div>
      </form>
      <ul className="flex flex-col gap-1.5">
        {invites.map((inv) => (
          <li
            key={inv.id}
            className={`panel-row ${inv.revoked ? 'opacity-50' : ''}`}
          >
            <div className="flex items-center gap-1">
              <code className="min-w-0 flex-1 truncate text-xs text-ink-1">
                {inv.code}
              </code>
              <button
                type="button"
                onClick={() => void copy(inv)}
                disabled={inv.revoked}
                className="btn btn-sm btn-secondary shrink-0"
                title="Copy invite code"
              >
                {copiedId === inv.id ? 'Copied' : 'Copy'}
              </button>
              <button
                type="button"
                onClick={() => void revoke(inv.id)}
                disabled={inv.revoked || busyId === inv.id}
                className="btn btn-sm btn-danger shrink-0"
                title="Revoke this invite code"
              >
                {inv.revoked
                  ? 'Revoked'
                  : busyId === inv.id
                    ? '…'
                    : 'Revoke'}
              </button>
            </div>
            <p className="meta-mono mt-1">
              {inv.initial_role} · {usesLabel(inv)} ·{' '}
              {inv.expires_at
                ? `expires ${new Date(inv.expires_at).toLocaleString()}`
                : 'never expires'}
            </p>
          </li>
        ))}
      </ul>
      {!loading && invites.length === 0 && (
        <p className="text-muted">
          No invites yet — create one above.
        </p>
      )}
    </section>
  );
}
