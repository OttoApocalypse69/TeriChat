import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import ConnectionIndicator from './components/ConnectionIndicator';
import ConversationList from './components/ConversationList';
import ConversationView from './components/ConversationView';
import LoginView from './components/LoginView';
import {
  ApiClient,
  apiBaseUrl,
  encodeOpaqueText,
  newClientMsgId,
} from './lib/api';
import { GatewayClient, type GatewayStatus } from './lib/gateway';
import { ChatStore, toChatMessage } from './lib/store';

export default function App() {
  const api = useMemo(() => new ApiClient(apiBaseUrl()), []);
  const storeRef = useRef(new ChatStore());
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

  const store = storeRef.current;
  const selected = store.conversations.find((c) => c.id === selectedId) ?? null;
  const messages = selectedId
    ? (store.messages.get(selectedId) ?? [])
    : [];

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
    setToken(null);
    setMeId('');
    setHandle('');
    setSelectedId(null);
    setStatus('disconnected');
    setError(null);
    bump();
  }

  async function openDm(peerHandle: string): Promise<void> {
    const conv = await api.createDm(peerHandle);
    storeRef.current.addConversation(conv);
    setSelectedId(conv.id);
    bump();
    await refreshHistory(conv.id);
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
          TeriChat <span className="text-zinc-500">· {handle}</span>
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
        <aside className="w-64 shrink-0 border-r border-zinc-800">
          <ConversationList
            conversations={store.conversations}
            selectedId={selectedId}
            onSelect={setSelectedId}
            onOpenDm={openDm}
          />
        </aside>
        <main className="min-w-0 flex-1" key={version}>
          <ConversationView
            conversation={selected}
            messages={messages}
            meId={meId}
            loading={loading}
            sending={sending}
            error={error}
            onSend={send}
          />
        </main>
      </div>
    </div>
  );
}
