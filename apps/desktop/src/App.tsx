import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import ChannelList from './components/ChannelList';
import ConnectionIndicator from './components/ConnectionIndicator';
import ConversationList from './components/ConversationList';
import ConversationView from './components/ConversationView';
import CreateWorkspace from './components/CreateWorkspace';
import InvitePanel from './components/InvitePanel';
import JoinWorkspace from './components/JoinWorkspace';
import LoginView from './components/LoginView';
import MemberPanel from './components/MemberPanel';
import WorkspaceList from './components/WorkspaceList';
import SessionPanel from './components/SessionPanel';
import WorkspaceActivity from './components/WorkspaceActivity';
import {
  BrandMark, ChatIcon, DevicesIcon, LogoutIcon, OpenLockIcon, PanelIcon, SearchIcon,
} from './components/icons';
import { avatarGradient } from './lib/avatar';
import {
  ApiClient,
  apiBaseUrl,
  encodeOpaqueText,
  newClientMsgId,
} from './lib/api';
import { GatewayClient, type GatewayOutboxEvent, type GatewayStatus } from './lib/gateway';
import {
  buildDmNotification,
  isWindowUnfocused,
  notifyDm,
  watchNotificationClick,
} from './lib/notifications';
import {
  ChatStore,
  gatewayEventInfo,
  parseMemberHandles,
  sortConversations,
  toChatMessage,
  unreadBadge,
  unreadCount,
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
  // null means never opened. Hiding an opened inventory keeps pending
  // revocations alive until this authenticated account is unmounted.
  const [sessionsOpen, setSessionsOpen] = useState<boolean | null>(null);
  const sessionsButton = useRef<HTMLButtonElement>(null);
  const sessionsRegion = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (sessionsOpen) sessionsRegion.current?.focus();
  }, [sessionsOpen]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [navigationQuery, setNavigationQuery] = useState('');
  // Presentation only: hiding a pane must not unmount its composer or panels.
  const [pane, setPane] = useState<'navigation' | 'conversation' | 'details'>('navigation');
  const navigationRef = useRef<HTMLElement>(null);
  const conversationRef = useRef<HTMLElement>(null);
  const detailsRef = useRef<HTMLElement>(null);
  useEffect(() => {
    if (pane === 'conversation') conversationRef.current?.focus();
    if (pane === 'navigation') (navigationRef.current?.querySelector<HTMLButtonElement>('button[aria-current="page"]') ?? navigationRef.current)?.focus();
    if (pane === 'details') detailsRef.current?.focus();
  }, [pane, selectedId]);
  // Bumped on every conversation selection change, so slow async work can
  // tell whether the user navigated elsewhere while it was pending.
  const navigationRevision = useRef(0);
  function openConversation(id: string) {
    navigationRevision.current += 1;
    setSelectedId(id);
    setPane('conversation');
  }
  const [loading, setLoading] = useState(false);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [wsLoading, setWsLoading] = useState(false);
  const [wsError, setWsError] = useState<string | null>(null);
  const [chLoading, setChLoading] = useState(false);
  const workspaceListRevision = useRef(0);

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
    const revision = ++workspaceListRevision.current;
    setWsLoading(true);
    setWsError(null);
    try {
      const rows = await api.listWorkspaces();
      if (!live.current || workspaceListRevision.current !== revision) return;
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
      if (live.current && workspaceListRevision.current === revision) setWsError(err instanceof Error ? err.message : 'workspaces failed');
    } finally {
      if (live.current && workspaceListRevision.current === revision) setWsLoading(false);
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

  // Toast clicks focus the main window (registered once per session).
  useEffect(() => {
    void watchNotificationClick();
  }, []);

  // Gateway lifecycle: connect on login, resume with last event id.
  useEffect(() => {
    if (!live.current) return;

    const onEvent = (event: GatewayOutboxEvent) => {
      if (!live.current) return;
      // Snapshot focus at arrival: a DM that lands while the window is
      // unfocused earns one toast after its text loads (see below).
      const wasUnfocused = isWindowUnfocused();
      // An event for a conversation this client has never listed (a group or
      // DM someone just created with us) needs the list to name and show it.
      const eventConversation = gatewayEventInfo(event)?.conversationId;
      const unknown = eventConversation !== undefined
        && !storeRef.current.conversations.some(c => c.id === eventConversation);
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
        void refreshHistory(info.conversationId).then(() => {
          // Exactly one toast per incoming message: only first-seen events
          // (`fresh`) notify, never redeliveries, and only the fetched row
          // matching this event (never a neighbor's text).
          if (!live.current || !wasUnfocused || !fresh) return;
          const arrived = storeRef.current.conversations.find(
            (c) => c.id === info.conversationId,
          ) ?? null;
          const row = (storeRef.current.messages.get(info.conversationId) ?? [])
            .find((m) => m.id === info.messageId);
          if (!row) return;
          const content = buildDmNotification(meId, arrived, row);
          if (!content) return;
          void notifyDm(content, { unfocused: wasUnfocused }).catch(() => {
            // Messaging already updated; a toast failure must stay silent.
          });
        }, () => {
          // refreshHistory handles fetch errors internally; this guards the
          // toast tail against unexpected throws without breaking messaging.
        });
        const conv = storeRef.current.conversations.find(
          c => c.id === info.conversationId,
        );
        if (unknown || (conv?.kind === 'dm' && !conv.peer_handle)) void refreshConversations();
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

  function sessionEnded() {
    if (!live.current) return;
    live.current = false;
    gatewayRef.current?.close();
    onLogout();
  }

  async function openDm(peerHandle: string): Promise<void> {
    const conv = await api.createDm(peerHandle);
    if (!live.current) return;
    storeRef.current.addConversation(conv);
    openConversation(conv.id);
    bump();
    await refreshConversations();
    await refreshHistory(conv.id);
  }

  /** Create a group from free-form handles, then land in it with names resolved. */
  async function createGroup(memberInput: string): Promise<void> {
    const handles = parseMemberHandles(memberInput, handle);
    if (handles.length === 0) throw new Error('Add at least one other person’s handle.');
    const revision = navigationRevision.current;
    const conv = await api.createGroup(handles);
    if (!live.current) return;
    storeRef.current.addConversation(conv);
    // Never yank a user (and their draft) out of a conversation they opened
    // while the group was being created; the new group is listed either way.
    if (navigationRevision.current === revision) openConversation(conv.id);
    bump();
    await refreshConversations();
    await refreshHistory(conv.id);
  }

  function selectWorkspace(id: string): void {
    wsStoreRef.current.selectWorkspace(id);
    bump();
  }

  async function createWorkspace(name: string): Promise<void> {
    const created = await api.createWorkspace(name);
    if (!live.current) return;
    // A list begun before creation must not erase the new workspace.
    workspaceListRevision.current += 1;
    setWsLoading(false);
    setWsError(null);
    wsStoreRef.current.setWorkspaces([
      ...wsStoreRef.current.workspaces.filter(workspace => workspace.id !== created.id), created,
    ]);
    wsStoreRef.current.selectWorkspace(created.id);
    navigationRevision.current += 1;
    setSelectedId(null);
    bump();
    // The discarded initial list may contain other memberships not yet loaded.
    // Read again after the write, retaining the selected created workspace.
    await refreshWorkspaces();
  }

  const handleMyRole = useCallback((workspaceId: string, role: string) => {
    if (!live.current) return;
    wsStoreRef.current.setWorkspaces(wsStoreRef.current.workspaces.map(workspace =>
      workspace.id === workspaceId ? { ...workspace, my_role: role } : workspace));
    bump();
  }, [bump]);

  /** Redeem an invite code and land in the workspace (select + channels). */
  async function joinByCode(code: string): Promise<string> {
    const joined = await api.joinWorkspace(code.trim());
    if (!live.current) return joined.id;
    workspaceListRevision.current += 1;
    setWsLoading(false);
    const rows = [
      ...wsStoreRef.current.workspaces.filter((w) => w.id !== joined.id),
      joined,
    ];
    wsStoreRef.current.setWorkspaces(rows);
    wsStoreRef.current.selectWorkspace(joined.id);
    bump();
    await refreshWorkspaces();
    await refreshChannels(joined.id);
    return joined.id;
  }

  /** Drop a left workspace from local state and clear its selection. */
  function handleLeftWorkspace(workspaceId: string): void {
    if (!live.current) return;
    workspaceListRevision.current += 1;
    setWsLoading(false);
    wsStoreRef.current.setWorkspaces(
      wsStore.workspaces.filter((w) => w.id !== workspaceId),
    );
    if (wsStoreRef.current.selectedWorkspaceId === workspaceId) {
      wsStoreRef.current.selectWorkspace(null);
    }
    if (!wsStoreRef.current.selectedWorkspaceId) setPane('navigation');
    bump();
  }

  function selectChannel(channelId: string): void {
    wsStoreRef.current.selectChannel(channelId);
    const channel = wsStoreRef.current.selectedChannel();
    if (channel) {
      storeRef.current.addConversation(channelToConversation(channel));
      openConversation(channel.conversation_id);
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
      if (wsStoreRef.current.selectedWorkspaceId !== workspaceId) return;
      wsStoreRef.current.selectChannel(channel.id);
      openConversation(channel.conversation_id);
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
      // The server advances the sender's marker with the send.
      storeRef.current.setReadMarker(sent.conversation_id, sent.seq);
      bump();
    } finally {
      if (live.current) setSending(false);
    }
  }

  useEffect(() => {
    if (selectedId) void refreshHistory(selectedId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

  // ---- Private unread state (never shared with other members) ----
  const unreadByConversation = new Map(
    store.conversations.map(c => [c.id, unreadCount(c, store.messages.get(c.id), meId)]),
  );
  const unreadWorkspaceIds = new Set(wsStore.workspaces
    .filter(w => wsStore.channelsFor(w.id).some(ch => (unreadByConversation.get(ch.conversation_id) ?? 0) > 0))
    .map(w => w.id));

  // The "new" divider sits where the marker was when the conversation opened,
  // and only when something was unread then; reading must not move it.
  const [unreadSnapshot, setUnreadSnapshot] = useState<{ id: string; seq: number } | null>(null);
  useEffect(() => {
    const seq = selected?.last_read_seq;
    const unread = selectedId ? (unreadByConversation.get(selectedId) ?? 0) : 0;
    setUnreadSnapshot(selectedId && seq != null && unread > 0 ? { id: selectedId, seq } : null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId]);

  // Re-check visibility when the window regains focus or the tab is shown.
  const [attentionTick, setAttentionTick] = useState(0);
  useEffect(() => {
    const poke = () => setAttentionTick(tick => tick + 1);
    window.addEventListener('focus', poke);
    document.addEventListener('visibilitychange', poke);
    return () => {
      window.removeEventListener('focus', poke);
      document.removeEventListener('visibilitychange', poke);
    };
  }, []);

  // A conversation is read only while someone can see it: its pane has
  // layout, the tab is visible, the window has focus and sessions are closed.
  const readInFlight = useRef(new Map<string, number>());
  const selectedLatestSeq = selectedId ? store.maxSeq(selectedId) : 0;
  useEffect(() => {
    if (!selected || selected.last_read_seq == null || selectedLatestSeq <= selected.last_read_seq) return;
    if (sessionsOpen || document.visibilityState === 'hidden' || !document.hasFocus()) return;
    if ((conversationRef.current?.getClientRects().length ?? 0) === 0) return;
    const id = selected.id;
    const target = selectedLatestSeq;
    if ((readInFlight.current.get(id) ?? -1) >= target) return;
    readInFlight.current.set(id, target);
    void api.markRead(id, target).then(marker => {
      if (!live.current) return;
      storeRef.current.setReadMarker(id, marker.last_read_seq);
      bump();
    }, () => {
      // Unconfirmed: the next message, focus change or selection retries.
    }).finally(() => {
      if (readInFlight.current.get(id) === target) readInFlight.current.delete(id);
    });
  }, [api, bump, selected, selectedLatestSeq, sessionsOpen, pane, attentionTick]);

  const baseTitle = useRef(document.title);
  const totalUnread = [...unreadByConversation.values()].reduce((sum, n) => sum + n, 0);
  useEffect(() => {
    document.title = totalUnread > 0 ? `(${unreadBadge(totalUnread)}) ${baseTitle.current}` : baseTitle.current;
  }, [totalUnread]);
  useEffect(() => {
    const base = baseTitle.current;
    return () => { document.title = base; };
  }, []);

  const closeSessions = () => { setSessionsOpen(false); sessionsButton.current?.focus(); };
  return (
    <div className="chat-shell" data-pane={pane} data-sessions-open={sessionsOpen === true}>
      <header className="app-header">
        <span className="topbar-brand">
          <span className="topbar-mark"><BrandMark /></span>
          <span className="brand-name">UnknownChat</span>
          <span className="topbar-phase tag tag-line">Alpha 0</span>
        </span>
        <span className="topbar-actions">
          <button ref={sessionsButton} type="button" aria-expanded={sessionsOpen ?? false} aria-controls="account-sessions"
            className="btn btn-ghost" onClick={() => setSessionsOpen(open => !open)}><DevicesIcon />Sessions</button>
          <button
            type="button"
            onClick={logout}
            className="btn btn-ghost"
          >
            <LogoutIcon />Log out
          </button>
          <span className="topbar-divider" aria-hidden />
          <span className="topbar-account">
            <span className="avatar avatar-28" aria-hidden style={{ background: avatarGradient(handle) }}>{handle.slice(0, 1).toUpperCase()}</span>
            <span className="account-handle">@{handle}</span>
          </span>
        </span>
      </header>
      {sessionsOpen !== null && <div id="account-sessions" hidden={!sessionsOpen} ref={sessionsRegion} tabIndex={-1}
        onKeyDown={event => {
          if (event.key === 'Escape') closeSessions();
        }}>
        <button className="nav-action" onClick={closeSessions}>Close sessions</button>
        <SessionPanel api={api} onSessionEnded={sessionEnded} />
      </div>}
      <div className="chat-layout">
        <nav className="workspace-rail" aria-label="Workspace switcher">
          <button type="button" className="rail-tile rail-home" title="Conversations" aria-label="Show conversations" onClick={() => setPane('navigation')}>
            <span className="rail-tile-rim" aria-hidden />
            <span className="rail-tile-face" aria-hidden><ChatIcon size={20} /></span>
          </button>
          <span className="rail-divider" aria-hidden />
          <WorkspaceList
            workspaces={wsStore.workspaces}
            unreadWorkspaceIds={unreadWorkspaceIds}
            selectedWorkspaceId={selectedWorkspaceId}
            loading={wsLoading}
            error={wsError}
            onSelect={id => { selectWorkspace(id); setPane('navigation'); }}
            onRetry={() => void refreshWorkspaces()}
          />
          <span className="rail-version rail-label" title="Alpha demo">α0</span>
        </nav>
        <aside ref={navigationRef} tabIndex={-1} aria-label="Conversations and workspaces" className="chat-navigation">
          <div className="navigation-heading">
            <h1>
              <span className="display-m">{selectedWorkspace?.name ?? 'Conversations'}</span>
              {selectedWorkspace && <span className="navigation-kind">WORKSPACE</span>}
            </h1>
            <span className="meta-mono">{selectedWorkspace ? `you are ${selectedWorkspace.my_role}` : 'Direct messages'}</span>
          </div>
          {wsError && <p role="alert" className="text-alert mx-4 mt-3">{wsError}</p>}
          <label className="navigation-search">
            <SearchIcon size={14} />
            <input type="search" aria-label="Filter conversations and channels" placeholder="Find a conversation…" value={navigationQuery} onChange={event => setNavigationQuery(event.target.value)} />
          </label>
          {(selectedWorkspace || selected) && <div className="navigation-shortcuts">
            {selectedWorkspace && <button type="button" className="details-toggle nav-row" aria-controls="workspace-details" onClick={() => setPane('details')}><PanelIcon />Workspace details</button>}
            {selected && <button type="button" className="mobile-only nav-action" onClick={() => setPane('conversation')}>Return to conversation</button>}
          </div>}
          <div className="shrink-0">
            <ChannelList
              filter={navigationQuery}
              unreadByConversation={unreadByConversation}
              key={selectedWorkspaceId ?? 'no-workspace'}
              workspaceName={selectedWorkspace?.name ?? null}
              channels={channels}
              selectedChannelId={selected?.kind === 'channel' ? wsStore.selectedChannelId : null}
              loading={chLoading}
              error={channelsError}
              onSelect={selectChannel}
              onCreateChannel={createChannel}
            />
          </div>
          <div className="shrink-0">
            <ConversationList
              filter={navigationQuery}
              meId={meId}
              conversations={dmConversations}
              messagesByConversation={store.messages}
              selectedId={selectedId}
              onSelect={openConversation}
              onOpenDm={openDm}
              onCreateGroup={createGroup}
            />
          </div>
          <div className="workspace-actions">
            <h2 className="label-mono px-2">Make room for your people</h2>
            <div className="workspace-actions-card">
              <CreateWorkspace onCreate={createWorkspace} />
              <JoinWorkspace onJoin={joinByCode} />
            </div>
          </div>
          <div className="navigation-account">
            <span className="avatar avatar-32" aria-hidden style={{ background: avatarGradient(handle) }}>{handle.slice(0, 1).toUpperCase()}</span>
            <span className="min-w-0"><strong className="block truncate">@{handle}</strong><span className="meta-mono">Alpha 0 · It sends bro</span></span>
          </div>
        </aside>
        <main ref={conversationRef} tabIndex={-1} aria-label="Conversation" className="chat-main" key={selectedId ?? 'none'}>
          <ConversationView
            headerActions={<div className="pane-toolbar">
            <button type="button" className="mobile-only nav-action" onClick={() => setPane('navigation')}>← Back to conversations</button>
            {selectedWorkspace && <button type="button" className="details-toggle btn btn-ghost" aria-controls="workspace-details" aria-expanded={pane === 'details'}
              onClick={() => setPane(pane === 'details' ? 'conversation' : 'details')}><PanelIcon />Workspace details</button>}
          </div>}
            isActivePane={pane === 'conversation'}
            conversation={selected}
            messages={messages}
            meId={meId}
            meHandle={handle}
            loading={loading}
            sending={sending}
            error={error}
            onSend={send}
            title={selected?.kind === 'channel' ? channelTitle : null}
            unreadAfterSeq={unreadSnapshot?.id === selectedId ? unreadSnapshot.seq : null}
          />
        </main>
        {selectedWorkspace && (
          <aside ref={detailsRef} tabIndex={-1} id="workspace-details" aria-label="Workspace details" className="workspace-details">
            <div className="details-header">
              <button type="button" className="details-toggle nav-action" onClick={() => setPane(selected ? 'conversation' : 'navigation')}>← Back</button>
              <h2 className="truncate">{selectedWorkspace.name}</h2>
              <span className="tag tag-line ml-auto">{selectedWorkspace.my_role}</span>
            </div>
            <MemberPanel
              key={`members-${selectedWorkspace.id}`}
              api={api}
              workspaceId={selectedWorkspace.id}
              myRole={selectedWorkspace.my_role}
              meId={meId}
              onLeft={handleLeftWorkspace}
              onMyRole={handleMyRole}
            />
            <WorkspaceActivity api={api} workspaceId={selectedWorkspace.id} meId={meId} />
            <InvitePanel
              key={`invites-${selectedWorkspace.id}`}
              api={api}
              workspaceId={selectedWorkspace.id}
              myRole={selectedWorkspace.my_role}
            />
          </aside>
        )}
      </div>
      <footer className="system-bar">
        <ConnectionIndicator status={status} />
        <span className="system-bar-trust"><OpenLockIcon size={13} /><strong>Not end-to-end encrypted</strong><span className="system-bar-detail"> · demo plaintext</span></span>
        <span className="system-bar-end">Alpha 0 · It sends bro</span>
      </footer>
    </div>
  );
}
