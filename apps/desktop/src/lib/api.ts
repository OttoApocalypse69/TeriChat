// Typed HTTP client for the UnknownChat Alpha 0 API (auth / DM / send / history).
//
// Transport only: envelope bytes are opaque base64 pass-through. No crypto,
// key handling, or sync authority lives here.
//
// NOTE: inside the Tauri shell, fetch comes from @tauri-apps/plugin-http
// (proxied through Rust), not the webview. WebView2 enforces CORS and the
// Alpha backend serves no CORS headers, so window.fetch fails from the
// desktop shell. The plugin path is capability-gated (see
// src-tauri/capabilities/default.json: loopback only). In a plain browser
// there is no Tauri bridge, so same-origin window.fetch is used instead
// (the web deployment serves API and UI from one origin, so no CORS).
import { fetch as tauriFetch } from '@tauri-apps/plugin-http';

/// True when running inside the Tauri shell (native bridge present).
export function isTauriShell(): boolean {
  return (
    typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
  );
}

const browserFetch: typeof fetch = (...args) =>
  globalThis.fetch(...args);

/// Transport fetch: Rust-proxied in the shell, same-origin in browsers.
function transportFetch(
  ...args: Parameters<typeof fetch>
): ReturnType<typeof fetch> {
  return (isTauriShell() ? tauriFetch : browserFetch)(...args);
}

export interface UserBody {
  id: string;
  handle: string;
  email: string;
  display_name: string;
  created_at: string;
}

export interface LoginResponse {
  token: string;
  session_id: string;
  user_id: string;
  expires_at: string;
  user_handle: string;
}

export interface ConversationBody {
  id: string;
  kind: string;
  members: string[];
}

/** GET /v1/conversations entry: caller-scoped, with the DM peer resolved. */
export interface ConversationSummaryBody {
  id: string;
  kind: string;
  members: string[];
  peer_handle: string | null;
  peer_display_name: string | null;
  last_seq: number | null;
  last_sent_at: string | null;
}

export interface MessageBody {
  id: string;
  conversation_id: string;
  sender_id: string;
  seq: number;
  ciphertext_b64: string;
  nonce_b64?: string | null;
  client_msg_id: string;
  sent_at: string;
  deduped: boolean;
}

export interface WorkspaceBody {
  id: string;
  name: string;
  owner_id: string;
  my_role: string;
  created_at: string;
  updated_at: string;
}

export interface ChannelBody {
  id: string;
  workspace_id: string;
  conversation_id: string;
  name: string;
  kind: string;
  created_by: string;
  created_at: string;
}

export interface WorkspaceMemberBody {
  user_id: string;
  handle: string;
  display_name: string;
  role: string;
  joined_at: string;
}

export interface WorkspaceMembersPage {
  members: WorkspaceMemberBody[];
  next_cursor: string | null;
}

export interface SessionBody {
  id: string;
  device_id: string | null;
  created_at: string;
  expires_at: string;
  is_current: boolean;
}
export interface SessionsPage {
  sessions: SessionBody[];
  next_cursor: string | null;
}
export interface WorkspaceStatsBody {
  user_id: string;
  workspace_id: string;
  message_count: number;
  last_message_at: string | null;
}
export interface ChannelStatsBody {
  channel_id: string;
  conversation_id: string;
  name: string;
  message_count: number;
  last_message_at: string | null;
}
export interface ChannelStatsPage {
  channels: ChannelStatsBody[];
  next_cursor: string | null;
}

/** Attachment row returned by the upload route (bytes fetched separately). */
export interface AttachmentBody {
  id: string;
  conversation_id: string;
  filename: string;
  mime: string;
  size_bytes: number;
  sha256: string;
  created_at: string;
}

/** Client-side attachment ref embedded in the message JSON payload. */
export interface AttachmentRef {
  id: string;
  filename: string;
  mime: string;
  size_bytes: number;
  sha256: string;
}

/** Client message content: text plus optional attachment refs. */
export interface ChatContent {
  text: string;
  attachments: AttachmentRef[];
}

/** Max attachment bytes the client will upload (mirrors the server 10 MiB cap). */
export const MAX_ATTACHMENT_BYTES = 10 * 1024 * 1024;

/** Max attachment refs embedded in one message (client-enforced). */
export const MAX_ATTACHMENTS_PER_MESSAGE = 5;

/**
 * Parse a received message payload into chat content.
 * Legacy messages carry a bare string; attachment messages carry
 * `{text, attachments}`; anything else degrades to empty text.
 */
export function parseChatContent(payload: string): ChatContent {
  const decoded = decodeOpaqueText(payload);
  if (decoded === payload && !isBase64Envelope(payload)) {
    return { text: payload, attachments: [] };
  }
  if (!decoded.startsWith('{')) return { text: decoded, attachments: [] };
  try {
    const parsed = JSON.parse(decoded) as {
      text?: unknown;
      attachments?: unknown;
    };
    if (typeof parsed.text !== 'string') return { text: decoded, attachments: [] };
    const attachments = Array.isArray(parsed.attachments)
      ? (parsed.attachments as Record<string, unknown>[]).flatMap((entry) =>
          typeof entry.id === 'string' &&
          typeof entry.filename === 'string' &&
          typeof entry.mime === 'string' &&
          typeof entry.size_bytes === 'number' &&
          typeof entry.sha256 === 'string'
            ? [
                {
                  id: entry.id,
                  filename: entry.filename,
                  mime: entry.mime,
                  size_bytes: entry.size_bytes,
                  sha256: entry.sha256,
                },
              ]
            : [],
        )
      : [];
    return { text: parsed.text, attachments };
  } catch {
    return { text: decoded, attachments: [] };
  }
}

/**
 * Encode chat content for the envelope. Plain-text messages stay bare
 * strings; messages with attachments become `{text, attachments}` JSON.
 */
export function encodeChatContent(text: string, attachments: AttachmentRef[]): string {
  if (attachments.length === 0) return text;
  return JSON.stringify({ text, attachments });
}

/** True when the payload looks like a STANDARD-base64 envelope. */
function isBase64Envelope(payload: string): boolean {
  const trimmed = payload.trim();
  return (
    trimmed.length > 0 &&
    trimmed.length % 4 === 0 &&
    /^[A-Za-z0-9+/]*={0,2}$/.test(trimmed)
  );
}

export interface InviteBody {
  id: string;
  workspace_id: string;
  code: string;
  created_by: string;
  initial_role: string;
  expires_at: string | null;
  max_uses: number | null;
  uses: number;
  revoked: boolean;
  created_at: string;
}

export interface AuditBody {
  id: string;
  workspace_id: string;
  actor_id: string;
  action: string;
  target_id: string | null;
  detail: Record<string, unknown>;
  created_at: string;
}

export class ApiError extends Error {
  status: number;
  code: string;
  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

export function apiBaseUrl(): string {
  const raw =
    (import.meta as unknown as { env?: Record<string, string | undefined> }).env
      ?.VITE_API_URL ?? '';
  return raw.trim() === '' ? 'http://127.0.0.1:3001' : raw.replace(/\/$/, '');
}

async function parseError(res: Response): Promise<ApiError> {
  let code = 'request_failed';
  let message = `request failed (${res.status})`;
  try {
    const body = (await res.json()) as {
      error?: { code?: string; message?: string };
    };
    if (body?.error?.code) code = body.error.code;
    if (body?.error?.message) message = body.error.message;
  } catch {
    // Keep the generic message when the body is not JSON.
  }
  return new ApiError(res.status, code, message);
}

export class ApiClient {
  private base: string;
  private token: string | null;

  constructor(baseUrl?: string, token?: string | null) {
    this.base = (baseUrl ?? apiBaseUrl()).replace(/\/$/, '');
    this.token = token ?? null;
  }

  setToken(token: string | null): void {
    this.token = token;
  }

  private async req<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<T> {
    const headers: Record<string, string> = {
      'content-type': 'application/json',
    };
    if (this.token) headers.authorization = `Bearer ${this.token}`;
    const res = await transportFetch(`${this.base}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!res.ok) throw await parseError(res);
    // Add-member / ban return 201 with an empty body, and several member
    // routes return 204: treat any empty success body as undefined instead
    // of failing JSON parsing.
    if (res.status === 204 || res.status === 205) return undefined as T;
    const text = await res.text();
    if (text.trim() === '') return undefined as T;
    return JSON.parse(text) as T;
  }

  register(input: {
    handle: string;
    email: string;
    display_name: string;
    password: string;
  }): Promise<UserBody> {
    return this.req<UserBody>('POST', '/v1/auth/register', input);
  }

  login(handle: string, password: string): Promise<LoginResponse> {
    return this.req<LoginResponse>('POST', '/v1/auth/login', {
      handle,
      password,
    });
  }

  logout(): Promise<{ status: string; session_id: string }> {
    return this.req('POST', '/v1/auth/logout', {});
  }

  listSessions(after?: string): Promise<SessionsPage> {
    const query = new URLSearchParams({ limit: '25' });
    if (after !== undefined) query.set('after', after);
    return this.req('GET', `/v1/auth/sessions?${query}`);
  }

  revokeSession(id: string): Promise<void> {
    return this.req('DELETE', `/v1/auth/sessions/${encodeURIComponent(id)}`);
  }

  workspaceStats(workspaceId: string): Promise<WorkspaceStatsBody> {
    return this.req('GET', `/v1/workspaces/${encodeURIComponent(workspaceId)}/stats/me`);
  }

  channelStats(workspaceId: string, after?: string): Promise<ChannelStatsPage> {
    const query = new URLSearchParams({ limit: '25' });
    if (after !== undefined) query.set('after', after);
    return this.req('GET', `/v1/workspaces/${encodeURIComponent(workspaceId)}/stats/me/channels?${query}`);
  }

  createDm(peer_handle: string): Promise<ConversationBody> {
    return this.req<ConversationBody>('POST', '/v1/conversations/dm', {
      peer_handle,
    });
  }

  createGroup(member_handles: string[]): Promise<ConversationBody> {
    return this.req<ConversationBody>('POST', '/v1/conversations', {
      member_handles,
    });
  }

  /** Caller-scoped conversation list with DM peers + last positions. */
  listConversations(): Promise<ConversationSummaryBody[]> {
    return this.req<ConversationSummaryBody[]>('GET', '/v1/conversations');
  }

  sendMessage(input: {
    conversation_id: string;
    client_msg_id: string;
    ciphertext_b64: string;
    nonce_b64?: string | null;
  }): Promise<MessageBody> {
    return this.req<MessageBody>('POST', '/v1/messages', input);
  }

  history(
    conversation_id: string,
    since_seq?: number,
    limit?: number,
  ): Promise<MessageBody[]> {
    const q = new URLSearchParams({ conversation_id });
    if (since_seq !== undefined) q.set('since_seq', String(since_seq));
    if (limit !== undefined) q.set('limit', String(limit));
    return this.req<MessageBody[]>('GET', `/v1/messages?${q.toString()}`);
  }

  /**
   * Upload raw attachment bytes under a conversation. Filename and mime
   * travel as query params (the body is the file bytes, not JSON).
   *
   * Transport split: the Tauri shell MUST use the Rust-proxied plugin
   * fetch (WebView2 enforces CORS on XHR and the API serves no CORS
   * headers — same reason `req` uses transportFetch). Plain browsers use
   * XHR for upload-progress events. The plugin path offers no progress
   * surface, so shell uploads report indeterminate progress.
   */
  async uploadAttachment(
    conversationId: string,
    file: File,
    onProgress?: (loaded: number, total: number) => void,
  ): Promise<AttachmentBody> {
    if (file.size > MAX_ATTACHMENT_BYTES) {
      throw new ApiError(
        413,
        'payload_too_large',
        `attachment exceeds ${MAX_ATTACHMENT_BYTES} bytes`,
      );
    }
    const query = new URLSearchParams({
      filename: file.name,
      mime: file.type || 'application/octet-stream',
    });
    const url = `${this.base}/v1/conversations/${conversationId}/attachments?${query.toString()}`;
    const headers: Record<string, string> = {
      'content-type': 'application/octet-stream',
    };
    if (this.token) headers.authorization = `Bearer ${this.token}`;
    if (!isTauriShell()) {
      return this.uploadViaXhr(url, headers, file, onProgress);
    }
    // Shell: plugin fetch serializes the body through the Rust bridge
    // (no progress events); the staged UI shows indeterminate motion.
    const res = await transportFetch(url, {
      method: 'POST',
      headers,
      body: file,
    });
    if (!res.ok) throw await parseError(res);
    return (await res.json()) as AttachmentBody;
  }

  /** Browser-only upload path with progress events (same-origin, no CORS issue). */
  private uploadViaXhr(
    url: string,
    headers: Record<string, string>,
    file: File,
    onProgress?: (loaded: number, total: number) => void,
  ): Promise<AttachmentBody> {
    return new Promise<AttachmentBody>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('POST', url);
      for (const [name, value] of Object.entries(headers)) {
        xhr.setRequestHeader(name, value);
      }
      if (xhr.upload && onProgress) {
        xhr.upload.onprogress = (event) => {
          if (event.lengthComputable) onProgress(event.loaded, event.total);
        };
      }
      xhr.onload = () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          try {
            resolve(JSON.parse(xhr.responseText) as AttachmentBody);
          } catch {
            reject(new ApiError(xhr.status, 'request_failed', 'bad upload response'));
          }
        } else {
          let code = 'request_failed';
          let message = `request failed (${xhr.status})`;
          try {
            const body = JSON.parse(xhr.responseText) as {
              error?: { code?: string; message?: string };
            };
            if (body?.error?.code) code = body.error.code;
            if (body?.error?.message) message = body.error.message;
          } catch {
            // Keep the generic message when the body is not JSON.
          }
          reject(new ApiError(xhr.status, code, message));
        }
      };
      xhr.onerror = () =>
        reject(new ApiError(0, 'request_failed', 'upload failed'));
      xhr.send(file);
    });
  }

  /** Download an attachment's original bytes as a Blob. */
  async downloadAttachment(id: string): Promise<{ blob: Blob; filename: string; mime: string }> {
    const headers: Record<string, string> = {};
    if (this.token) headers.authorization = `Bearer ${this.token}`;
    const res = await transportFetch(
      `${this.base}/v1/attachments/${encodeURIComponent(id)}`,
      { headers },
    );
    if (!res.ok) throw await parseError(res);
    const blob = await res.blob();
    const disposition = res.headers.get('content-disposition') ?? '';
    const filename = /filename="([^"]*)"/.exec(disposition)?.[1] ?? 'download';
    return {
      blob,
      filename,
      mime: res.headers.get('content-type') ?? 'application/octet-stream',
    };
  }

  listWorkspaces(): Promise<WorkspaceBody[]> {
    return this.req<WorkspaceBody[]>('GET', '/v1/workspaces');
  }

  createWorkspace(name: string): Promise<WorkspaceBody> {
    return this.req<WorkspaceBody>('POST', '/v1/workspaces', { name });
  }

  listMembers(workspaceId: string, after?: string): Promise<WorkspaceMembersPage> {
    const query = new URLSearchParams({ limit: '100' });
    if (after !== undefined) query.set('after', after);
    return this.req<WorkspaceMembersPage>(
      'GET', `/v1/workspaces/${workspaceId}/members?${query.toString()}`,
    );
  }

  listChannels(workspaceId: string): Promise<ChannelBody[]> {
    return this.req<ChannelBody[]>(
      'GET',
      `/v1/workspaces/${workspaceId}/channels`,
    );
  }

  createChannel(workspaceId: string, name: string): Promise<ChannelBody> {
    return this.req<ChannelBody>(
      'POST',
      `/v1/workspaces/${workspaceId}/channels`,
      { name },
    );
  }

  // --- Phase 3: workspace invites + members (server-owned, transport only) ---

  /** Redeem an invite code. Returns the joined workspace (with my_role). */
  joinWorkspace(code: string): Promise<WorkspaceBody> {
    return this.req<WorkspaceBody>('POST', '/v1/workspaces/join', { code });
  }

  createInvite(
    workspaceId: string,
    input: {
      initial_role?: string;
      expires_in_secs?: number;
      max_uses?: number;
    },
  ): Promise<InviteBody> {
    return this.req<InviteBody>(
      'POST',
      `/v1/workspaces/${workspaceId}/invites`,
      input,
    );
  }

  listInvites(workspaceId: string): Promise<InviteBody[]> {
    return this.req<InviteBody[]>(
      'GET',
      `/v1/workspaces/${workspaceId}/invites`,
    );
  }

  revokeInvite(workspaceId: string, inviteId: string): Promise<void> {
    return this.req<void>(
      'DELETE',
      `/v1/workspaces/${workspaceId}/invites/${inviteId}`,
    );
  }

  /** Direct-add by handle. Resolves 201 with an empty body. */
  addMember(
    workspaceId: string,
    input: { user_handle: string; role: string },
  ): Promise<void> {
    return this.req<void>(
      'POST',
      `/v1/workspaces/${workspaceId}/members`,
      input,
    );
  }

  setMemberRole(
    workspaceId: string,
    userId: string,
    role: string,
  ): Promise<void> {
    return this.req<void>(
      'PATCH',
      `/v1/workspaces/${workspaceId}/members/${userId}`,
      { role },
    );
  }

  kickMember(workspaceId: string, userId: string): Promise<void> {
    return this.req<void>(
      'DELETE',
      `/v1/workspaces/${workspaceId}/members/${userId}`,
    );
  }

  leaveWorkspace(workspaceId: string): Promise<void> {
    return this.req<void>('POST', `/v1/workspaces/${workspaceId}/leave`);
  }

  /** Ban resolves 201 with an empty body. */
  banMember(workspaceId: string, userId: string, reason?: string): Promise<void> {
    return this.req<void>('POST', `/v1/workspaces/${workspaceId}/bans`, {
      user_id: userId,
      ...(reason !== undefined ? { reason } : {}),
    });
  }

  unbanMember(workspaceId: string, userId: string): Promise<void> {
    return this.req<void>(
      'DELETE',
      `/v1/workspaces/${workspaceId}/bans/${userId}`,
    );
  }

  listAudit(workspaceId: string, limit = 100): Promise<AuditBody[]> {
    const q = new URLSearchParams({ limit: String(limit) });
    return this.req<AuditBody[]>(
      'GET',
      `/v1/workspaces/${workspaceId}/audit?${q.toString()}`,
    );
  }
}

// --- Opaque envelope transport helpers (base64 pass-through, NOT crypto) ---

/** Encode demo plaintext to the STANDARD-base64 envelope the server stores. */
export function encodeOpaqueText(plain: string): string {
  const bytes = new TextEncoder().encode(plain);
  let bin = '';
  for (let i = 0; i < bytes.length; i += 8192) {
    bin += String.fromCharCode(...bytes.subarray(i, i + 8192));
  }
  return btoa(bin);
}

/** Decode an opaque envelope for demo display; falls back to raw on error. */
export function decodeOpaqueText(ciphertextB64: string): string {
  try {
    const bin = atob(ciphertextB64.trim());
    const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return ciphertextB64;
  }
}

/** Random idempotency key for POST /v1/messages. Any UUID works server-side. */
export function newClientMsgId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }
  // Fallback for non-secure contexts: random hex in UUID shape.
  const h = () =>
    Math.floor(Math.random() * 0xffffffff)
      .toString(16)
      .padStart(8, '0');
  const a = h();
  const b = h();
  return `${a.slice(0, 8)}-${b.slice(0, 4)}-4${b.slice(5, 8)}-8${a.slice(1, 4)}-${a}${b}`.slice(
    0,
    36,
  );
}
