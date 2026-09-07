// Workspace/channel selection store (Phase 2, Alpha-sized).
//
// Transport only: workspaces/channels are server-owned records fetched over
// the Tauri HTTP plugin. No crypto, key handling, or sync authority lives
// here. Channel messages reuse the existing conversation message flow via
// ChannelBody.conversation_id.

import { ApiError, type ChannelBody, type WorkspaceBody } from './api';

/** True when switching conversations must reset the composer draft. */
export function shouldClearComposer(
  prevConversationId: string | null,
  nextConversationId: string | null,
): boolean {
  return prevConversationId !== nextConversationId;
}

/**
 * Human-readable error for channel operations. 403 from the server means
 * the caller lacks the workspace permission (e.g. MANAGE_CHANNELS for
 * create) — surface that plainly instead of a bare status code.
 */
export function friendlyChannelError(err: unknown): string {
  if (err instanceof ApiError && err.status === 403) {
    return `not permitted (403): ${err.message}`;
  }
  return err instanceof Error ? err.message : 'request failed';
}

/** Conversation row backing a channel (message flow keys on this id). */
export function channelToConversation(channel: ChannelBody): {
  id: string;
  kind: string;
  members: string[];
} {
  return { id: channel.conversation_id, kind: 'channel', members: [] };
}

export class WorkspaceStore {
  workspaces: WorkspaceBody[] = [];
  selectedWorkspaceId: string | null = null;
  channelsByWorkspace = new Map<string, ChannelBody[]>();
  channelsErrorByWorkspace = new Map<string, string | null>();
  selectedChannelId: string | null = null;

  setWorkspaces(workspaces: WorkspaceBody[]): void {
    this.workspaces = [...workspaces].sort((a, b) =>
      a.name.localeCompare(b.name),
    );
    // Drop selection when it no longer exists; otherwise keep it.
    if (
      this.selectedWorkspaceId !== null &&
      !this.workspaces.some((w) => w.id === this.selectedWorkspaceId)
    ) {
      this.selectedWorkspaceId = null;
      this.selectedChannelId = null;
    }
  }

  selectWorkspace(id: string | null): void {
    if (this.selectedWorkspaceId !== id) {
      this.selectedWorkspaceId = id;
      // Channel selection belongs to the previous workspace — clear it so a
      // stale highlight can never point at another workspace's channel.
      this.selectedChannelId = null;
    }
  }

  selectedWorkspace(): WorkspaceBody | null {
    return (
      this.workspaces.find((w) => w.id === this.selectedWorkspaceId) ?? null
    );
  }

  setChannels(workspaceId: string, channels: ChannelBody[]): void {
    this.channelsByWorkspace.set(
      workspaceId,
      [...channels].sort((a, b) => a.name.localeCompare(b.name)),
    );
    this.channelsErrorByWorkspace.set(workspaceId, null);
    if (
      this.selectedChannelId !== null &&
      !channels.some((c) => c.id === this.selectedChannelId)
    ) {
      this.selectedChannelId = null;
    }
  }

  setChannelsError(workspaceId: string, message: string | null): void {
    this.channelsErrorByWorkspace.set(workspaceId, message);
  }

  channelsFor(workspaceId: string): ChannelBody[] {
    return this.channelsByWorkspace.get(workspaceId) ?? [];
  }

  channelsErrorFor(workspaceId: string): string | null {
    return this.channelsErrorByWorkspace.get(workspaceId) ?? null;
  }

  selectChannel(id: string | null): void {
    this.selectedChannelId = id;
  }

  selectedChannel(): ChannelBody | null {
    if (this.selectedWorkspaceId === null || this.selectedChannelId === null) {
      return null;
    }
    return (
      this.channelsFor(this.selectedWorkspaceId).find(
        (c) => c.id === this.selectedChannelId,
      ) ?? null
    );
  }

  /** conversation_id backing the selected channel (message-flow key). */
  selectedChannelConversationId(): string | null {
    return this.selectedChannel()?.conversation_id ?? null;
  }
}
