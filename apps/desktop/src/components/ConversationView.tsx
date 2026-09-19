import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import {
  parseChatContent,
  type AttachmentRef,
} from '../lib/api';
import type { GatewayStatus } from '../lib/gateway';
import {
  avatarInitial,
  conversationLabel,
  conversationSublabel,
  dayKey,
  dayLabel,
  formatClockTime,
  senderLabel,
  type ChatConversation,
  type ChatMessage,
} from '../lib/store';
import ConnectionIndicator from './ConnectionIndicator';

interface Props {
  conversation: ChatConversation | null;
  messages: ChatMessage[];
  meId: string;
  status: GatewayStatus;
  loading: boolean;
  sending: boolean;
  error: string | null;
  onSend: (text: string) => Promise<void>;
  onSendAttachments: (files: File[]) => Promise<void>;
  onDownload: (ref: AttachmentRef) => Promise<void>;
  fetchAttachmentBytes: (ref: AttachmentRef) => Promise<Blob>;
  title?: string | null;
  isActivePane?: boolean;
}

export default function ConversationView({
  conversation,
  messages,
  meId,
  status,
  loading,
  sending,
  error,
  onSend,
  onSendAttachments,
  onDownload,
  fetchAttachmentBytes,
  title,
  isActivePane = true,
}: Props) {
  const [draft, setDraft] = useState('');
  const draftRevision = useRef(0);
  const [sendError, setSendError] = useState<string | null>(null);
  const [staged, setStaged] = useState<File[]>([]);
  const [uploadProgress, setUploadProgress] = useState<number | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Switching conversations (DM <-> DM, DM <-> channel, channel <-> channel)
  // must never leak the previous composer draft into the new conversation.
  const conversationId = conversation?.id ?? null;
  useEffect(() => {
    setDraft('');
    setSendError(null);
    setStaged([]);
    setUploadProgress(null);
  }, [conversationId]);

  // Hidden mounted panes have no scroll box. Reconcile pending follows when
  // navigation or CSS breakpoint layout reveals this conversation, not on focus.
  const pendingTail = useRef(false);
  useLayoutEffect(() => {
    pendingTail.current = true;
  }, [messages.length, conversationId]);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const followPendingTail = () => {
      if (pendingTail.current && el.clientHeight > 0) {
        el.scrollTop = el.scrollHeight;
        pendingTail.current = false;
      }
    };
    followPendingTail();
    // ResizeObserver also fires when display:none becomes a real layout box.
    // Consumed pending state leaves readers alone on ordinary viewport resizes.
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(followPendingTail);
    observer.observe(el);
    return () => observer.disconnect();
  }, [messages.length, conversationId, isActivePane]);

  async function submit(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    if (sending || (!draft.trim() && staged.length === 0)) return;
    const revision = draftRevision.current;
    setSendError(null);
    try {
      if (staged.length > 0) {
        const files = staged;
        setStaged([]);
        setUploadProgress(0);
        try {
          await onSendAttachments(files);
        } finally {
          setUploadProgress(null);
        }
        // Text alongside staged files goes as its own message below.
        if (!draft.trim()) {
          if (draftRevision.current === revision) setDraft('');
          return;
        }
      }
      await onSend(draft.trim());
      if (draftRevision.current === revision) setDraft('');
    } catch (err) {
      setSendError(err instanceof Error ? err.message : 'send failed');
    }
  }

  function stageFiles(list: FileList | null): void {
    if (!list) return;
    setSendError(null);
    const picked = Array.from(list).slice(0, 5 - staged.length);
    const oversize = picked.filter((f) => f.size > 10 * 1024 * 1024);
    if (oversize.length > 0) {
      setSendError(
        `${oversize[0].name} exceeds 10 MiB and was not staged`,
      );
    }
    setStaged((prev) => [...prev, ...picked.filter((f) => f.size <= 10 * 1024 * 1024)].slice(0, 5));
    if (fileInput.current) fileInput.current.value = '';
  }

  if (!conversation) {
    return (
      <div className="conversation-empty flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-sm text-zinc-400">
        <span className="text-lg font-semibold text-zinc-100">Your conversations, in one place</span>
        <span>Select a conversation or open a DM to start chatting.</span>
      </div>
    );
  }

  const heading = title ?? conversationLabel(conversation);
  const sub = title ? conversation.kind : conversationSublabel(conversation);

  let lastDay = '';
  return (
    <div className="conversation-view flex min-h-0 flex-1 flex-col">
      <div className="conversation-header flex shrink-0 items-center gap-3 border-b border-zinc-800 px-5 py-4">
        <span
          aria-hidden
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-zinc-700 text-xs font-bold text-zinc-200"
        >
          {avatarInitial(conversation)}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-semibold text-zinc-100">
            {heading}
          </span>
          {sub && (
            <span className="block truncate text-[11px] text-zinc-500">
              {sub}
            </span>
          )}
        </span>
        <span className="ml-auto shrink-0">
          <ConnectionIndicator status={status} />
        </span>
      </div>
      <p className="plaintext-warning shrink-0 border-b border-amber-900/40 bg-amber-950/20 px-5 py-2 text-xs leading-relaxed text-amber-200/90">
        Alpha demo: envelopes carry demo plaintext — not end-to-end encrypted.
      </p>
      <div ref={scrollRef} className="message-history min-h-0 flex-1 space-y-3 overflow-y-auto p-5" aria-label="Message history">
        {loading && <p className="text-sm text-zinc-400">Loading history…</p>}
        {messages.map((m) => {
          const divider =
            dayKey(m.sent_at) !== lastDay ? dayLabel(m.sent_at) : null;
          lastDay = dayKey(m.sent_at);
          const mine = m.sender_id === meId;
          const content = parseChatContent(m.ciphertext_b64);
          return (
            <div key={m.id}>
              {divider && (
                <p className="py-1 text-center text-[11px] font-medium text-zinc-500">
                  — {divider} —
                </p>
              )}
              <div
                className={`message-bubble max-w-[80%] rounded-xl px-4 py-3 text-sm leading-relaxed ${
                  mine
                    ? 'ml-auto bg-emerald-900 text-emerald-50'
                    : 'bg-zinc-800 text-zinc-100'
                }`}
              >
                {content.text && (
                  <p className="whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
                    {content.text}
                  </p>
                )}
                {content.attachments.map((ref) => (
                  <AttachmentRow
                    key={ref.id}
                    ref={ref}
                    mine={mine}
                    onDownload={() => void onDownload(ref)}
                    fetchBytes={() => fetchAttachmentBytes(ref)}
                  />
                ))}
                <p className="message-meta mt-2 text-[10px] text-zinc-300">
                  #{m.seq} {senderLabel(meId, m.sender_id, conversation)}
                  {formatClockTime(m.sent_at)
                    ? ` · ${formatClockTime(m.sent_at)}`
                    : ''}
                </p>
              </div>
            </div>
          );
        })}
        {!loading && messages.length === 0 && (
          <p className="text-sm text-zinc-400">
            No messages yet — say bro.
          </p>
        )}
      </div>
      {error && <p role="alert" className="px-4 py-1 text-sm text-red-400">{error}</p>}
      {sendError && <p role="alert" className="px-4 py-1 text-sm text-red-400">{sendError}</p>}
      {staged.length > 0 && (
        <div className="shrink-0 border-t border-zinc-800 px-4 pt-2" aria-label="Staged files">
          {staged.map((file, index) => (
            <p key={`${file.name}-${index}`} className="flex items-center justify-between gap-2 text-xs text-zinc-300">
              <span className="truncate">📎 {file.name} ({formatBytes(file.size)})</span>
              <button
                type="button"
                className="shrink-0 text-zinc-500 hover:text-zinc-200"
                aria-label={`Remove ${file.name}`}
                onClick={() => setStaged((prev) => prev.filter((_, i) => i !== index))}
              >
                ✕
              </button>
            </p>
          ))}
          {uploadProgress !== null && (
            <div className="my-1 h-1 overflow-hidden rounded bg-zinc-800" role="progressbar" aria-label="Upload progress">
              <div className="h-full bg-emerald-400" style={{ width: `${Math.round(uploadProgress * 100)}%` }} />
            </div>
          )}
        </div>
      )}
      <form onSubmit={submit} className="message-composer flex shrink-0 gap-2 border-t border-zinc-800 p-4">
        <input
          ref={fileInput}
          type="file"
          multiple
          className="hidden"
          aria-label="Attach files"
          accept="image/*,audio/*,video/*,.pdf,.zip"
          onChange={(e) => stageFiles(e.target.files)}
        />
        <button
          type="button"
          aria-label="Attach files"
          title="Attach files (images, GIFs, audio, video, PDF, ZIP — max 10 MiB each)"
          disabled={sending}
          className="shrink-0 rounded-lg bg-zinc-800 px-3 py-3 text-sm disabled:opacity-40"
          onClick={() => fileInput.current?.click()}
        >
          📎
        </button>
        <input
          aria-label="Message"
          className="min-w-0 flex-1 rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-3 text-sm"
          placeholder="Message (demo plaintext → opaque envelope)"
          value={draft}
          onChange={(e) => {
            draftRevision.current += 1;
            setDraft(e.target.value);
          }}
        />
        <button
          type="submit"
          disabled={sending || (!draft.trim() && staged.length === 0)}
          className="shrink-0 rounded-lg bg-emerald-400 px-4 py-3 text-sm font-semibold text-emerald-950 disabled:opacity-40"
        >
          {sending ? '…' : 'Send'}
        </button>
      </form>
    </div>
  );
}

/** Format a byte count for staged-file rows. */
function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}

/**
 * One attachment inside a message bubble. Images render an inline thumbnail
 * (fetched lazily on click to keep history scrolls cheap); other files
 * render a download row. Thumbnails revoke their object URLs on unmount.
 */
function AttachmentRow({
  ref,
  mine,
  onDownload,
  fetchBytes,
}: {
  ref: AttachmentRef;
  mine: boolean;
  onDownload: () => void;
  fetchBytes: () => Promise<Blob>;
}) {
  const [thumb, setThumb] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [loadingPreview, setLoadingPreview] = useState(false);
  const isImage = ref.mime.toLowerCase().startsWith('image/');

  useEffect(() => {
    return () => {
      if (thumb) URL.revokeObjectURL(thumb);
    };
  }, [thumb]);

  async function showPreview(): Promise<void> {
    if (loadingPreview || thumb) return;
    setLoadingPreview(true);
    try {
      const blob = await fetchBytes();
      setThumb(URL.createObjectURL(blob));
    } catch {
      setFailed(true);
    } finally {
      setLoadingPreview(false);
    }
  }

  if (!isImage) {
    return (
      <button
        type="button"
        onClick={onDownload}
        className={`mt-2 flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-xs ${
          mine ? 'bg-emerald-950 text-emerald-100' : 'bg-zinc-900 text-zinc-200'
        }`}
      >
        <span aria-hidden>📄</span>
        <span className="min-w-0 flex-1 truncate">{ref.filename}</span>
        <span className="shrink-0 opacity-70">{formatBytes(ref.size_bytes)}</span>
      </button>
    );
  }
  if (thumb) {
    return (
      <button type="button" onClick={onDownload} className="mt-2 block max-w-full">
        <img
          src={thumb}
          alt={ref.filename}
          className="max-h-64 max-w-full rounded-lg object-contain"
          onError={() => setFailed(true)}
        />
      </button>
    );
  }
  return (
    <button
      type="button"
      disabled={failed || loadingPreview}
      onClick={() => void showPreview()}
      className={`mt-2 flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-xs ${
        mine ? 'bg-emerald-950 text-emerald-100' : 'bg-zinc-900 text-zinc-200'
      }`}
    >
      <span aria-hidden>🖼️</span>
      <span className="min-w-0 flex-1 truncate">
        {failed
          ? `${ref.filename} (preview failed — click to download)`
          : loadingPreview
            ? `${ref.filename} — loading preview…`
            : `${ref.filename} — show preview`}
      </span>
      <span className="shrink-0 opacity-70">{formatBytes(ref.size_bytes)}</span>
    </button>
  );
}
