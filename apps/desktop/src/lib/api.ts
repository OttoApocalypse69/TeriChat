// Typed HTTP client for the TeriChat Alpha 0 API (auth / DM / send / history).
//
// Transport only: envelope bytes are opaque base64 pass-through. No crypto,
// key handling, or sync authority lives here.

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
    const res = await fetch(`${this.base}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!res.ok) throw await parseError(res);
    if (res.status === 204) return undefined as T;
    return (await res.json()) as T;
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
