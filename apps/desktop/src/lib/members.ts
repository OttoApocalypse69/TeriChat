// Phase 3 member/invite helpers (desktop-only, Alpha).
//
// Mirrors the exact server contract in apps/server/src/workspaces.rs:
// rank order owner > admin > moderator > member > guest; a non-owner actor
// must strictly outrank both the target's current role and the granted role
// (peers are untouchable; the owner bypasses). Invites can never grant the
// owner role. Permission grants mirror `role_has`: ManageMembers +
// KickMembers reach moderator, ManageRoles + BanMembers need admin/owner.
//
// Transport only: no crypto, key handling, or sync authority lives here.

import { ApiError } from './api';

export type RoleName = 'owner' | 'admin' | 'moderator' | 'member' | 'guest';

export const ROLE_NAMES: RoleName[] = [
  'owner',
  'admin',
  'moderator',
  'member',
  'guest',
];

export const ROLE_RANK: Record<RoleName, number> = {
  guest: 0,
  member: 1,
  moderator: 2,
  admin: 3,
  owner: 4,
};

/** Parse a server/client role name (trimmed, case-insensitive). */
export function normalizeRole(raw: unknown): RoleName | null {
  if (typeof raw !== 'string') return null;
  const v = raw.trim().toLowerCase();
  return (ROLE_NAMES as string[]).includes(v) ? (v as RoleName) : null;
}

/** Fail closed: unknown roles gate as guest. */
export function roleOrGuest(raw: unknown): RoleName {
  return normalizeRole(raw) ?? 'guest';
}

export type WorkspacePermission =
  | 'ManageMembers'
  | 'ManageRoles'
  | 'KickMembers'
  | 'BanMembers';

/** Client mirror of the server `role_has` grant for the gated permissions. */
export function roleHas(role: RoleName, perm: WorkspacePermission): boolean {
  switch (role) {
    case 'owner':
    case 'admin':
      return true;
    case 'moderator':
      return perm === 'ManageMembers' || perm === 'KickMembers';
    case 'member':
    case 'guest':
      return false;
  }
}

/** Manager-only invite panel gate (create/list/revoke need ManageMembers). */
export function canUseInvitePanel(myRole: RoleName): boolean {
  return roleHas(myRole, 'ManageMembers');
}

/**
 * Client mirror of the server `check_rank`: the owner bypasses; anyone else
 * must strictly outrank the target's current role and (for grants) the new
 * role. Unknown target roles fail closed.
 */
export function rankAllows(
  actor: RoleName,
  target: RoleName | null,
  next?: RoleName,
): boolean {
  if (actor === 'owner') return true;
  if (target === null) return false;
  if (ROLE_RANK[target] >= ROLE_RANK[actor]) return false;
  if (next !== undefined && ROLE_RANK[next] >= ROLE_RANK[actor]) return false;
  return true;
}

export interface Gate {
  ok: boolean;
  /** Human reason, for tooltips and disabled states. Empty when ok. */
  reason: string;
}

/** Direct-add / invite-grant gate (server can never grant owner by invite). */
export function gateGrant(actor: RoleName, grant: RoleName): Gate {
  if (!roleHas(actor, 'ManageMembers')) {
    return {
      ok: false,
      reason: 'requires a manager role (moderator or above)',
    };
  }
  if (grant === 'owner') {
    return { ok: false, reason: 'owner cannot be granted (server rule)' };
  }
  if (!rankAllows(actor, 'guest', grant)) {
    return {
      ok: false,
      reason: `${grant} is not below your rank (${actor})`,
    };
  }
  return { ok: true, reason: '' };
}

/** Roles the actor may offer in the invite/add forms (never owner). */
export function grantableRoles(actor: RoleName): RoleName[] {
  return (['guest', 'member', 'moderator', 'admin'] as RoleName[]).filter(
    (r) => gateGrant(actor, r).ok,
  );
}

export function gateSetRole(
  actor: RoleName,
  target: RoleName | null,
  next: RoleName,
): Gate {
  if (!roleHas(actor, 'ManageRoles')) {
    return { ok: false, reason: 'changing roles requires admin or owner' };
  }
  if (target === null) {
    return { ok: false, reason: 'role unknown — cannot rank-check' };
  }
  if (!rankAllows(actor, target, next)) {
    return {
      ok: false,
      reason: 'you can only set roles below your own rank',
    };
  }
  return { ok: true, reason: '' };
}

export function gateKick(actor: RoleName, target: RoleName | null): Gate {
  if (!roleHas(actor, 'KickMembers')) {
    return { ok: false, reason: 'kicking requires moderator or above' };
  }
  if (target === null) {
    return { ok: false, reason: 'role unknown — cannot rank-check' };
  }
  if (!rankAllows(actor, target)) {
    return {
      ok: false,
      reason: 'you can only kick roles below your own rank',
    };
  }
  return { ok: true, reason: '' };
}

export function gateBan(actor: RoleName, target: RoleName | null): Gate {
  if (!roleHas(actor, 'BanMembers')) {
    return { ok: false, reason: 'banning requires admin or owner' };
  }
  if (target !== null && !rankAllows(actor, target)) {
    return {
      ok: false,
      reason: 'you can only ban roles below your own rank',
    };
  }
  return { ok: true, reason: '' };
}

/**
 * Friendly error for invite/member operations. 403s name the permission
 * wall; 400/404 carry the exact server reason (invite invalid/expired,
 * already a member, last owner, banned) so it stays actionable.
 */
export function friendlyMemberError(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.status === 403) return `not permitted (403): ${err.message}`;
    return err.message;
  }
  return err instanceof Error ? err.message : 'request failed';
}

// --- Invite-code join state machine (pure, tested) ---

export type JoinState =
  | { phase: 'idle' }
  | { phase: 'joining'; code: string }
  | { phase: 'joined'; workspaceId: string }
  | { phase: 'error'; message: string };

export type JoinEvent =
  | { type: 'submit'; code: string }
  | { type: 'ok'; workspaceId: string }
  | { type: 'fail'; message: string }
  | { type: 'reset' };

export function reduceJoinState(state: JoinState, event: JoinEvent): JoinState {
  switch (event.type) {
    case 'submit': {
      const code = event.code.trim();
      if (code === '') {
        return { phase: 'error', message: 'paste an invite code first' };
      }
      if (state.phase === 'joining') return state;
      return { phase: 'joining', code };
    }
    case 'ok':
      return state.phase === 'joining'
        ? { phase: 'joined', workspaceId: event.workspaceId }
        : state;
    case 'fail':
      return state.phase === 'joining'
        ? { phase: 'error', message: event.message }
        : state;
    case 'reset':
      return { phase: 'idle' };
  }
}
