import { describe, expect, it } from 'vitest';
import { ApiError } from '../api';
import {
  WorkspaceStore,
  channelToConversation,
  friendlyChannelError,
  shouldClearComposer,
} from '../workspaces';
import type { ChannelBody, WorkspaceBody } from '../api';

function ws(over: Partial<WorkspaceBody> & { id: string }): WorkspaceBody {
  return {
    name: `ws-${over.id}`,
    owner_id: 'owner-1',
    my_role: 'member',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...over,
  };
}

function ch(
  over: Partial<ChannelBody> & { id: string; workspace_id: string },
): ChannelBody {
  return {
    conversation_id: `conv-${over.id}`,
    name: `ch-${over.id}`,
    kind: 'text',
    created_by: 'owner-1',
    created_at: '2026-01-01T00:00:00Z',
    ...over,
  };
}

describe('workspace selection', () => {
  it('selects a workspace and clears stale channel selection', () => {
    const store = new WorkspaceStore();
    store.setWorkspaces([ws({ id: 'w1' }), ws({ id: 'w2' })]);
    store.selectWorkspace('w1');
    store.setChannels('w1', [ch({ id: 'c1', workspace_id: 'w1' })]);
    store.selectChannel('c1');
    expect(store.selectedChannel()?.id).toBe('c1');

    store.selectWorkspace('w2');
    expect(store.selectedWorkspaceId).toBe('w2');
    expect(store.selectedChannelId).toBeNull();
    expect(store.selectedChannel()).toBeNull();
  });

  it('drops selection when the workspace list no longer contains it', () => {
    const store = new WorkspaceStore();
    store.setWorkspaces([ws({ id: 'w1' })]);
    store.selectWorkspace('w1');
    store.setWorkspaces([ws({ id: 'w2' })]);
    expect(store.selectedWorkspaceId).toBeNull();
  });

  it('switching channels changes the backing conversation id', () => {
    const store = new WorkspaceStore();
    store.setWorkspaces([ws({ id: 'w1' })]);
    store.selectWorkspace('w1');
    const c1 = ch({ id: 'c1', workspace_id: 'w1' });
    const c2 = ch({ id: 'c2', workspace_id: 'w1' });
    store.setChannels('w1', [c1, c2]);

    store.selectChannel('c1');
    const first = store.selectedChannelConversationId();
    store.selectChannel('c2');
    const second = store.selectedChannelConversationId();
    expect(first).toBe('conv-c1');
    expect(second).toBe('conv-c2');
    expect(first).not.toBe(second);
    expect(channelToConversation(c2)).toMatchObject({
      id: 'conv-c2',
      kind: 'channel',
    });
  });
});

describe('channel switching clears composer state', () => {
  it('flags a draft reset whenever the conversation id changes', () => {
    expect(shouldClearComposer('conv-a', 'conv-b')).toBe(true);
    expect(shouldClearComposer('conv-a', 'conv-a')).toBe(false);
    expect(shouldClearComposer(null, 'conv-b')).toBe(true);
    expect(shouldClearComposer('conv-a', null)).toBe(true);
    expect(shouldClearComposer(null, null)).toBe(false);
  });
});

describe('403 surfacing on create-channel', () => {
  it('renders a plain not-permitted message for 403', () => {
    const err = new ApiError(
      403,
      'forbidden',
      'insufficient workspace permission',
    );
    expect(friendlyChannelError(err)).toContain('403');
    expect(friendlyChannelError(err)).toContain(
      'insufficient workspace permission',
    );
  });

  it('passes through non-403 errors unchanged', () => {
    expect(friendlyChannelError(new Error('boom'))).toBe('boom');
    const bad = new ApiError(400, 'bad_request', 'name too short');
    expect(friendlyChannelError(bad)).toBe('name too short');
  });
});
