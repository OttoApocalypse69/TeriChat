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
import { GatewayClient, type GatewayOutboxEvent, type GatewayStatus } from './lib/gateway';
import {
  ChatStore,
  gatewayEventInfo,
  sortConversations,
  toChatMessage,
} from './lib/store';
import {
  WorkspaceStore,
  channelToConversation,
  friendlyChannelError,
} from './lib/workspaces';

interface Session {
  token: string;
  meId: string;
  handle: string;
}

export default function App() {
  const loginApi = useMemo(() => new ApiClient(apiBaseUrl()), []);
  const [session, setSession] = useState<Session | null>(null);
  if (!session) {
    return (
      <LoginView
        api={loginApi}
        onAuthed={(token, meId, handle) => setSession({ token, meId, handle })}
      />
    );
  }
  // The entire authenticated tree, its stores and transport belong to one
  // login. Old async callbacks can never acquire the next account's state
  // or bearer token, including callbacks owned by workspace child panels.
  return (
    <AuthenticatedApp
      key={session.token}
      session={session}
      onLogout={() => setSession(null)}
    />
  );
}

function AuthenticatedApp({ session, onLogout }: {
  session: Session;
  onLogout: () => void;
}) {
  const { token, meId, handle } = session;
  const api = useMemo(() => new ApiClient(apiBaseUrl(), token), [token]);
  const storeRef = useRef(new ChatStore());
  const wsStoreRef = useRef(new WorkspaceStore());
  const gatewayRef = useRef<GatewayClient | null>(null);
  const [version, setVersion] = useState(0);
  const bump = useCallback(() => setVersion((v) => v + 1), []);


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

  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const historyJobs = useRef(
    new Map<string, { again: boolean; promise: Promise<void> }>(),
  );

  const refreshHistory = useCallback(
    (conversationId: string): Promise<void> => {
      if (!live.current) return Promise.resolve();
      const existing = historyJobs.current.get(conversationId);
      if (existing) {
        existing.again = true;
        return existing.promise;
      }
      const job = { again: false, promise: Promise.resolve() };
      historyJobs.current.set(conversationId, job);
      setLoading(true);
      setError(null);
      job.promise = (async () => {
        try {
          do {
            job.again = false;
            setError(null);
            try {
              let fullPage: boolean;
              do {
                const since = storeRef.current.historyCursor(conversationId);
                const page = await api.history(conversationId, since, 100);
                if (!live.current) return;
                storeRef.current.mergeHistoryPage(conversationId, page.map(toChatMessage));
                bump();
                fullPage = page.length === 100;
                if (fullPage && storeRef.current.historyCursor(conversationId) <= since) {
                  throw new Error('history did not advance');
                }
              } while (fullPage && live.current);
            } catch (err) {
              if (live.current) setError(err instanceof Error ? err.message : 'history failed');
            }
            // A reconnect/event queued during a failing request still earns
            // one retry. Without another trigger a persistent failure stops.
          } while (job.again && live.current);
        } finally {
          historyJobs.current.delete(conversationId);
          if (live.current) setLoading(historyJobs.current.size > 0);
        }
      })();
      return job.promise;
    },
    [api, bump],
  );

  const refreshConversations = useCallback(async () => {
    if (!live.current) return;
    try {
      const rows = await api.listConversations();
      if (!live.current) return;
      for (const row of rows) storeRef.current.addConversation(row);
      bump();
    } catch (err) {
      if (live.current) setError(err instanceof Error ? err.message : 'conversations failed');
    }
  }, [api, bump, token]);

  const refreshWorkspaces = useCallback(async () => {
    if (!live.current) return;
    setWsLoading(true);
    setWsError(null);
    try {
      const rows = await api.listWorkspaces();
      if (!live.current) return;
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
      if (live.current) setWsError(err instanceof Error ? err.message : 'workspaces failed');
    } finally {
      if (live.current) setWsLoading(false);
    }
  }, [api, bump, token]);

  const refreshChannels = useCallback(
    async (workspaceId: string) => {
      if (!live.current) return;
      setChLoading(true);
      try {
        const rows = await api.listChannels(workspaceId);
        if (!live.current) return;
        wsStoreRef.current.setChannels(workspaceId, rows);
        // Register channel conversations so gateway events + history reuse
        // the existing DM message flow keyed by conversation_id.
        for (const ch of rows) {
          storeRef.current.addConversation(channelToConversation(ch));
        }
        bump();
      } catch (err) {
        if (!live.current) return;
        wsStoreRef.current.setChannelsError(
          workspaceId,
          err instanceof Error ? err.message : 'channels failed',
        );
        bump();
      } finally {
        if (live.current) setChLoading(false);
      }
    },
    [api, bump, token],
  );

  // Gateway lifecycle: connect on login, resume with last event id.
  useEffect(() => {
    if (!live.current) return;

    const onEvent = (event: GatewayOutboxEvent) => {
      if (!live.current) return;
      const fresh = storeRef.current.applyGatewayEvent(event);
      const replay = fresh ? null : gatewayEventInfo(event);
      // Deduplicate event effects, not an unfinished history fetch.
      const info = fresh ?? (
        replay && storeRef.current.historyCursor(replay.conversationId) < replay.seq
          ? replay
          : null
      );
      bump();
      if (info) {
        // Events carry ids only; use the same serialized page drain as selection.
        void refreshHistory(info.conversationId);
        const conv = storeRef.current.conversations.find(
          c => c.id === info.conversationId,
        );
        if (conv?.kind === 'dm' && !conv.peer_handle) void refreshConversations();
      }
    };

    const gw = new GatewayClient({
      httpBase: apiBaseUrl(),
      token,
      getResumeAfter: () => storeRef.current.lastEventId,
      onStatus: (next) => {
        if (!live.current) return;
        setStatus(next);
        if (next === 'connected') {
          // Resume acknowledges event delivery, not successful history fetches.
          // Reconcile even if the server has no newer event to replay.
          void (async () => {
            await refreshConversations();
            for (const conv of storeRef.current.conversations) {
              if (!live.current) return;
              await refreshHistory(conv.id);
            }
          })();
        }
      },
      onEvent,
      onDuplicateEvent: onEvent,
    });
    gatewayRef.current = gw;
    gw.connect();
    return () => {
      gw.close();
      gatewayRef.current = null;
    };
  }, [api, bump, token, refreshHistory, refreshConversations]);

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

  function logout() {
    live.current = false;
    gatewayRef.current?.close();
    void api.logout().catch(() => undefined);
    onLogout();
  }

  async function openDm(peerHandle: string): Promise<void> {
    const conv = await api.createDm(peerHandle);
    if (!live.current) return;
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
    if (!live.current) return joined.id;
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
    if (!live.current) return;
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
      if (!live.current) return;
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
      if (!live.current) return;
      storeRef.current.mergeOutgoing(toChatMessage(sent));
      bump();
    } finally {
      if (live.current) setSending(false);
    }
  }

  useEffect(() => {
    if (selectedId) void refreshHistory(selectedId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

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
        <main className="min-w-0 flex-1" key={selectedId ?? 'none'}>
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
