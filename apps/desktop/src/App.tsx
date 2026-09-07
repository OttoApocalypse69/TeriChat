import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import ChannelList from './components/ChannelList';
import ConnectionIndicator from './components/ConnectionIndicator';
import ConversationList from './components/ConversationList';
import ConversationView from './components/ConversationView';
import InvitePanel from './components/InvitePanel';
import JoinWorkspace from './components/JoinWorkspace';
import LoginView from './components/LoginView';
import MemberPanel from './components/MemberPanel';
import WorkspaceList from './components/WorkspaceList';
import {
  ApiClient,
  apiBaseUrl,
  encodeOpaqueText,
  newClientMsgId,
} from './lib/api';
import { GatewayClient, type GatewayStatus } from './lib/gateway';
import { ChatStore, sortConversations, toChatMessage } from './lib/store';
import {
  WorkspaceStore,
  channelToConversation,
  friendlyChannelError,
} from './lib/workspaces';

export default function App() {
  const api = useMemo(() => new ApiClient(apiBaseUrl()), []);
  const storeRef = useRef(new ChatStore());
  const wsStoreRef = useRef(new WorkspaceStore());
  const gatewayRef = useRef<GatewayClient | null>(null);
  const [version, setVersion] = useState(0);
  const bump = useCallback(() => setVersion((v) => v + 1), []);

  const [token, setToken] = useState<string | null>(null);
  const [meId, setMeId] = useState('');
  const [handle, setHandle] = useState('');
  const [status, setStatus] = useState<GatewayStatus>('disconnected');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [wsLoading, setWsLoading] = useState(false);
  const [wsError, setWsError] = useState<string | null>(null);
  const [chLoading, setChLoading] = useState(false);

  const store = storeRef.current;
  const wsStore = wsStoreRef.current;
  const selected = store.conversations.find((c) => c.id === selectedId) ?? null;
  const messages = selectedId
    ? (store.messages.get(selectedId) ?? [])
    : [];

  // DM/group conversations only — channel rows live in the channel list and
  // reuse the same message flow via their conversation_id.
  const dmConversations = useMemo(
    () =>
      sortConversations(
        store.conversations.filter((c) => c.kind !== 'channel'),
        store.messages,
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [store.conversations, store.messages, version],
  );

  const selectedWorkspaceId = wsStore.selectedWorkspaceId;
  const selectedWorkspace = wsStore.selectedWorkspace();
  const channels = selectedWorkspaceId
    ? wsStore.channelsFor(selectedWorkspaceId)
    : [];
  const channelsError = selectedWorkspaceId
    ? wsStore.channelsErrorFor(selectedWorkspaceId)
    : null;
  const selectedChannel = wsStore.selectedChannel();
  const channelTitle = selectedChannel ? `#${selectedChannel.name}` : null;

  const refreshHistory = useCallback(
    async (conversationId: string) => {
      if (!token) return;
      setLoading(true);
      setError(null);
      try {
        const since = storeRef.current.maxSeq(conversationId);
        const page = await api.history(conversationId, since, 100);
        storeRef.current.mergeHistory(
          conversationId,
          page.map(toChatMessage),
        );
        bump();
      } catch (err) {
        setError(err instanceof Error ? err.message : 'history failed');
      } finally {
        setLoading(false);
      }
    },
    [api, bump, token],
  );

  const refreshConversations = useCallback(async () => {
    if (!token) return;
    try {
      const rows = await api.listConversations();
      for (const row of rows) storeRef.current.addConversation(row);
      bump();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'conversations failed');
    }
  }, [api, bump, token]);

  const refreshWorkspaces = useCallback(async () => {
    if (!token) return;
    setWsLoading(true);
    setWsError(null);
    try {
      const rows = await api.listWorkspaces();
      wsStoreRef.current.setWorkspaces(rows);
      // Auto-select the first workspace on first load for an Alpha-sized
      // one-click path into channels.
      if (
        wsStoreRef.current.selectedWorkspaceId === null &&
        rows.length > 0
      ) {
        const first = [...rows].sort((a, b) =>
          a.name.localeCompare(b.name),
        )[0];
        wsStoreRef.current.selectWorkspace(first.id);
      }
      bump();
    } catch (err) {
      setWsError(err instanceof Error ? err.message : 'workspaces failed');
    } finally {
      setWsLoading(false);
    }
  }, [api, bump, token]);

  const refreshChannels = useCallback(
    async (workspaceId: string) => {
      if (!token) return;
      setChLoading(true);
      try {
        const rows = await api.listChannels(workspaceId);
        wsStoreRef.current.setChannels(workspaceId, rows);
        // Register channel conversations so gateway events + history reuse
        // the existing DM message flow keyed by conversation_id.
        for (const ch of rows) {
          storeRef.current.addConversation(channelToConversation(ch));
        }
        bump();
      } catch (err) {
        wsStoreRef.current.setChannelsError(
          workspaceId,
          err instanceof Error ? err.message : 'channels failed',
        );
        bump();
      } finally {
        setChLoading(false);
      }
    },
    [api, bump, token],
  );

  // Gateway lifecycle: connect on login, resume with last event id.
  useEffect(() => {
    if (!token) return;
    api.setToken(token);
    const gw = new GatewayClient({
      httpBase: apiBaseUrl(),
      token,
      getResumeAfter: () => storeRef.current.lastEventId,
      onStatus: setStatus,
      onEvent: (event) => {
        const info = storeRef.current.applyGatewayEvent(event);
        bump();
        if (info) {
          // Events carry ids only — pull the opaque bytes via history.
          void (async () => {
            try {
              const since = storeRef.current.maxSeq(info.conversationId);
              const page = await api.history(info.conversationId, since, 100);
              storeRef.current.mergeHistory(
                info.conversationId,
                page.map(toChatMessage),
              );
              bump();
            } catch {
              // History retry happens on next event / selection.
            }
          })();
        }
      },
    });
    gatewayRef.current = gw;
    gw.connect();
    return () => {
      gw.close();
      gatewayRef.current = null;
    };
  }, [api, bump, token]);

  // Load conversations + workspaces once per login so DMs survive reload
  // with peer names (not UUIDs).
  useEffect(() => {
    if (token) {
      void refreshConversations();
      void refreshWorkspaces();
    }
  }, [refreshConversations, refreshWorkspaces, token]);

  // Load channels whenever the selected workspace changes.
  useEffect(() => {
    if (token && selectedWorkspaceId) void refreshChannels(selectedWorkspaceId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedWorkspaceId, token]);

  function onAuthed(nextToken: string, userId: string, userHandle: string) {
    api.setToken(nextToken);
    setToken(nextToken);
    setMeId(userId);
    setHandle(userHandle);
  }

  function logout() {
    gatewayRef.current?.close();
    void api.logout().catch(() => undefined);
    api.setToken(null);
    storeRef.current = new ChatStore();
    wsStoreRef.current = new WorkspaceStore();
    setToken(null);
    setMeId('');
    setHandle('');
    setSelectedId(null);
    setStatus('disconnected');
    setError(null);
    setWsError(null);
    bump();
  }

  async function openDm(peerHandle: string): Promise<void> {
    const conv = await api.createDm(peerHandle);
    storeRef.current.addConversation(conv);
    setSelectedId(conv.id);
    bump();
    await refreshConversations();
    await refreshHistory(conv.id);
  }

  function selectWorkspace(id: string): void {
    wsStoreRef.current.selectWorkspace(id);
    bump();
  }

  /** Redeem an invite code and land in the workspace (select + channels). */
  async function joinByCode(code: string): Promise<string> {
    const joined = await api.joinWorkspace(code.trim());
    const rows = [
      ...wsStoreRef.current.workspaces.filter((w) => w.id !== joined.id),
      joined,
    ];
    wsStoreRef.current.setWorkspaces(rows);
    wsStoreRef.current.selectWorkspace(joined.id);
    bump();
    await refreshChannels(joined.id);
    return joined.id;
  }

  /** Drop a left workspace from local state and clear its selection. */
  function handleLeftWorkspace(workspaceId: string): void {
    wsStoreRef.current.setWorkspaces(
      wsStore.workspaces.filter((w) => w.id !== workspaceId),
    );
    if (wsStoreRef.current.selectedWorkspaceId === workspaceId) {
      wsStoreRef.current.selectWorkspace(null);
    }
    bump();
  }

  function selectChannel(channelId: string): void {
    wsStoreRef.current.selectChannel(channelId);
    const channel = wsStoreRef.current.selectedChannel();
    if (channel) {
      storeRef.current.addConversation(channelToConversation(channel));
      setSelectedId(channel.conversation_id);
      bump();
    }
  }

  async function createChannel(name: string): Promise<void> {
    const workspaceId = wsStoreRef.current.selectedWorkspaceId;
    if (!workspaceId) throw new Error('select a workspace first');
    try {
      const channel = await api.createChannel(workspaceId, name);
      const current = wsStoreRef.current.channelsFor(workspaceId);
      wsStoreRef.current.setChannels(workspaceId, [...current, channel]);
      storeRef.current.addConversation(channelToConversation(channel));
      wsStoreRef.current.selectChannel(channel.id);
      setSelectedId(channel.conversation_id);
      bump();
      await refreshHistory(channel.conversation_id);
    } catch (err) {
      // Let ChannelList render the friendly (403-aware) message; rethrow so
      // its form error path triggers.
      throw new Error(friendlyChannelError(err));
    }
  }

  async function send(text: string): Promise<void> {
    if (!selected) return;
    setSending(true);
    try {
      const sent = await api.sendMessage({
        conversation_id: selected.id,
        client_msg_id: newClientMsgId(),
        ciphertext_b64: encodeOpaqueText(text),
      });
      storeRef.current.mergeOutgoing(toChatMessage(sent));
      bump();
    } finally {
      setSending(false);
    }
  }

  useEffect(() => {
    if (selectedId) void refreshHistory(selectedId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

  if (!token) return <LoginView api={api} onAuthed={onAuthed} />;

  return (
    <div className="flex h-full flex-col bg-zinc-950 text-zinc-100">
      <header className="flex items-center justify-between border-b border-zinc-800 px-3 py-2">
        <span className="text-sm font-semibold">
          UnknownChat <span className="text-zinc-500">· {handle}</span>
        </span>
        <span className="flex items-center gap-3">
          <ConnectionIndicator status={status} />
          <button
            type="button"
            onClick={logout}
            className="rounded bg-zinc-800 px-2 py-1 text-xs"
          >
            Log out
          </button>
        </span>
      </header>
      <div className="flex min-h-0 flex-1">
        <aside className="flex w-64 shrink-0 flex-col border-r border-zinc-800">
          <div className="max-h-48 shrink-0 overflow-y-auto">
            <WorkspaceList
              workspaces={wsStore.workspaces}
              selectedWorkspaceId={selectedWorkspaceId}
              loading={wsLoading}
              error={wsError}
              onSelect={selectWorkspace}
              onRetry={() => void refreshWorkspaces()}
            />
          </div>
          <div className="shrink-0 border-b border-zinc-800">
            <JoinWorkspace onJoin={joinByCode} />
          </div>
          <div className="max-h-64 shrink-0 overflow-y-auto">
            <ChannelList
              workspaceName={selectedWorkspace?.name ?? null}
              channels={channels}
              selectedChannelId={wsStore.selectedChannelId}
              loading={chLoading}
              error={channelsError}
              onSelect={selectChannel}
              onCreateChannel={createChannel}
            />
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto">
            <ConversationList
              conversations={dmConversations}
              messagesByConversation={store.messages}
              selectedId={selectedId}
              onSelect={setSelectedId}
              onOpenDm={openDm}
            />
          </div>
        </aside>
        <main className="min-w-0 flex-1" key={`${version}-${selectedId ?? 'none'}`}>
          <ConversationView
            conversation={selected}
            messages={messages}
            meId={meId}
            status={status}
            loading={loading}
            sending={sending}
            error={error}
            onSend={send}
            title={channelTitle}
          />
        </main>
        {selectedWorkspace && (
          <aside className="w-80 shrink-0 overflow-y-auto border-l border-zinc-800">
            <div className="border-b border-zinc-800 p-2">
              <h2 className="truncate text-[11px] font-semibold uppercase tracking-wide text-zinc-400">
                {selectedWorkspace.name} · {selectedWorkspace.my_role}
              </h2>
            </div>
            <MemberPanel
              key={`members-${selectedWorkspace.id}`}
              api={api}
              workspaceId={selectedWorkspace.id}
              myRole={selectedWorkspace.my_role}
              meId={meId}
              myHandle={handle}
              onLeft={handleLeftWorkspace}
            />
            <InvitePanel
              key={`invites-${selectedWorkspace.id}`}
              api={api}
              workspaceId={selectedWorkspace.id}
              myRole={selectedWorkspace.my_role}
            />
          </aside>
        )}
      </div>
    </div>
  );
}
