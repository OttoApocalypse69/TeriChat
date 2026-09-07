import { describe, expect, it } from 'vitest';
import { ApiError } from '../api';
import {
  MemberDirectory,
  canUseInvitePanel,
  friendlyMemberError,
  gateBan,
  gateGrant,
  gateKick,
  gateSetRole,
  grantableRoles,
  rankAllows,
  reduceJoinState,
  roleHas,
  type JoinState,
} from '../members';

describe('rank rule mirrors the server (owner bypass, peers untouchable)', () => {
  it('owner outranks everyone including owners', () => {
    expect(rankAllows('owner', 'owner', 'owner')).toBe(true);
    expect(rankAllows('owner', 'admin', 'admin')).toBe(true);
  });

  it('peers are untouchable for non-owners', () => {
    expect(rankAllows('admin', 'admin')).toBe(false);
    expect(rankAllows('moderator', 'moderator')).toBe(false);
    expect(rankAllows('admin', 'member', 'admin')).toBe(false);
    expect(rankAllows('moderator', 'member', 'moderator')).toBe(false);
  });

  it('strictly lower targets and grants pass', () => {
    expect(rankAllows('admin', 'moderator', 'moderator')).toBe(true);
    expect(rankAllows('moderator', 'member', 'guest')).toBe(true);
    expect(rankAllows('admin', 'member')).toBe(true);
  });

  it('higher targets always fail', () => {
    expect(rankAllows('moderator', 'admin')).toBe(false);
    expect(rankAllows('member', 'moderator')).toBe(false);
    expect(rankAllows('guest', 'member')).toBe(false);
  });

  it('unknown target roles fail closed', () => {
    expect(rankAllows('admin', null)).toBe(false);
    expect(rankAllows('owner', null)).toBe(true);
  });
});

describe('rank-aware manager action gates', () => {
  it('invite panel needs moderator or above', () => {
    expect(canUseInvitePanel('owner')).toBe(true);
    expect(canUseInvitePanel('admin')).toBe(true);
    expect(canUseInvitePanel('moderator')).toBe(true);
    expect(canUseInvitePanel('member')).toBe(false);
    expect(canUseInvitePanel('guest')).toBe(false);
  });

  it('grants never include owner and stay below the actor', () => {
    expect(grantableRoles('owner')).toEqual([
      'guest',
      'member',
      'moderator',
      'admin',
    ]);
    expect(grantableRoles('admin')).toEqual([
      'guest',
      'member',
      'moderator',
    ]);
    expect(grantableRoles('moderator')).toEqual(['guest', 'member']);
    expect(grantableRoles('member')).toEqual([]);
    expect(gateGrant('admin', 'owner').ok).toBe(false);
  });

  it('set-role needs admin/owner plus the rank rule', () => {
    expect(gateSetRole('moderator', 'member', 'guest').ok).toBe(false);
    expect(gateSetRole('admin', 'admin', 'member').ok).toBe(false);
    // Re-granting a below-rank role is a server-allowed no-op, not a peer touch.
    expect(gateSetRole('admin', 'moderator', 'moderator').ok).toBe(true);
    expect(gateSetRole('admin', 'moderator', 'member').ok).toBe(true);
    expect(gateSetRole('owner', 'admin', 'owner').ok).toBe(true);
  });

  it('kick reaches moderator but stays rank-aware', () => {
    expect(gateKick('member', 'guest').ok).toBe(false);
    expect(gateKick('moderator', 'moderator').ok).toBe(false);
    expect(gateKick('moderator', 'member').ok).toBe(true);
    expect(gateKick('admin', 'moderator').ok).toBe(true);
  });

  it('ban needs admin/owner plus the rank rule', () => {
    expect(gateBan('moderator', 'member').ok).toBe(false);
    expect(gateBan('admin', 'admin').ok).toBe(false);
    expect(gateBan('admin', 'moderator').ok).toBe(true);
    expect(roleHas('moderator', 'BanMembers')).toBe(false);
    expect(roleHas('moderator', 'KickMembers')).toBe(true);
  });

  it('disabled gates carry a human reason', () => {
    for (const gate of [
      gateGrant('member', 'member'),
      gateSetRole('admin', 'admin', 'member'),
      gateKick('moderator', 'admin'),
      gateBan('moderator', 'guest'),
    ]) {
      expect(gate.ok).toBe(false);
      expect(gate.reason.length).toBeGreaterThan(0);
    }
  });
});

describe('invite join state transitions', () => {
  const idle: JoinState = { phase: 'idle' };

  it('blank codes never start a join', () => {
    const next = reduceJoinState(idle, { type: 'submit', code: '   ' });
    expect(next).toEqual({ phase: 'error', message: expect.any(String) });
  });

  it('submit -> joining -> joined lands with the workspace id', () => {
    const joining = reduceJoinState(idle, { type: 'submit', code: '  abc ' });
    expect(joining).toEqual({ phase: 'joining', code: 'abc' });
    const joined = reduceJoinState(joining, {
      type: 'ok',
      workspaceId: 'ws-1',
    });
    expect(joined).toEqual({ phase: 'joined', workspaceId: 'ws-1' });
  });

  it('joining ignores a second submit and surfaces failures friendly', () => {
    const joining = reduceJoinState(idle, { type: 'submit', code: 'abc' });
    expect(reduceJoinState(joining, { type: 'submit', code: 'xyz' })).toBe(
      joining,
    );
    const failed = reduceJoinState(joining, {
      type: 'fail',
      message: 'invite is invalid, expired, or fully used',
    });
    expect(failed).toEqual({
      phase: 'error',
      message: 'invite is invalid, expired, or fully used',
    });
  });

  it('stray ok/fail events are ignored, reset returns to idle', () => {
    expect(reduceJoinState(idle, { type: 'ok', workspaceId: 'w' })).toBe(idle);
    const err = reduceJoinState(idle, { type: 'submit', code: '' });
    expect(reduceJoinState(err, { type: 'reset' })).toEqual({ phase: 'idle' });
  });
});

describe('403 surfacing on invite/member errors', () => {
  it('prefixes 403 with the permission wall', () => {
    const err = new ApiError(
      403,
      'forbidden',
      'insufficient workspace permission',
    );
    expect(friendlyMemberError(err)).toContain('403');
    expect(friendlyMemberError(err)).toContain(
      'insufficient workspace permission',
    );
  });

  it('passes server reasons through for 400/404 (invite, member, owner)', () => {
    expect(
      friendlyMemberError(
        new ApiError(404, 'not_found', 'invite is invalid, expired, or fully used'),
      ),
    ).toBe('invite is invalid, expired, or fully used');
    expect(
      friendlyMemberError(
        new ApiError(400, 'bad_request', 'user is already a member'),
      ),
    ).toBe('user is already a member');
    expect(
      friendlyMemberError(
        new ApiError(
          400,
          'bad_request',
          'workspace must keep at least one owner',
        ),
      ),
    ).toContain('at least one owner');
    expect(
      friendlyMemberError(
        new ApiError(403, 'forbidden', 'banned from this workspace'),
      ),
    ).toContain('banned from this workspace');
  });

  it('falls back for non-API errors', () => {
    expect(friendlyMemberError(new Error('boom'))).toBe('boom');
    expect(friendlyMemberError('nope')).toBe('request failed');
  });
});

describe('local member directory (no server list endpoint)', () => {
  function audit(
    action: string,
    target: string | null,
    detail: Record<string, unknown> = {},
  ) {
    return {
      id: `a-${action}-${target ?? 'none'}`,
      workspace_id: 'ws-1',
      actor_id: 'actor-1',
      action,
      target_id: target,
      detail,
      created_at: '2026-01-01T00:00:00Z',
    };
  }

  it('seeds self, folds audit seats, and sorts self first', () => {
    const dir = new MemberDirectory();
    dir.seedSelf('me', 'alice', 'admin');
    dir.mergeAudit([
      audit('member.added', 'u2', { role: 'member' }),
      audit('invite.accepted', 'u3'),
      audit('member.role_changed', 'u2', { from: 'member', to: 'moderator' }),
    ]);
    const rows = dir.list();
    expect(rows[0]).toMatchObject({ userId: 'me', role: 'admin' });
    expect(rows.find((r) => r.userId === 'u2')).toMatchObject({
      role: 'moderator',
    });
    // invite.accepted carries no role: the seat is known but role stays null.
    expect(rows.find((r) => r.userId === 'u3')).toMatchObject({ role: null });
  });

  it('removes kicked/left seats and tracks bans for unban', () => {
    const dir = new MemberDirectory();
    dir.seedSelf('me', 'alice', 'admin');
    dir.mergeAudit([
      audit('member.added', 'u2', { role: 'member' }),
      audit('member.banned', 'u2', { reason: '' }),
    ]);
    expect(dir.list().find((r) => r.userId === 'u2')).toMatchObject({
      banned: true,
    });
    dir.mergeAudit([audit('member.unbanned', 'u2', {})]);
    expect(dir.list().find((r) => r.userId === 'u2')).toMatchObject({
      banned: false,
    });
    dir.mergeAudit([audit('member.added', 'u9', { role: 'guest' })]);
    dir.mergeAudit([audit('member.kicked', 'u9', {})]);
    expect(dir.list().find((r) => r.userId === 'u9')).toBeUndefined();
  });
});
