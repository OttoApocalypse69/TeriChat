import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import {
  MAX_ATTACHMENT_BYTES,
  MAX_ATTACHMENTS_PER_MESSAGE,
  isAllowedAttachment,
  parseChatContent,
  type AttachmentRef,
  type MemberProfileBody,
} from '../lib/api';
import { avatarGradient } from '../lib/avatar';
import {
  avatarInitial,
  conversationLabel,
  conversationSublabel,
  dayKey,
  dayLabel,
  formatClockTime,
  memberProfile,
  senderLabel,
  type ChatConversation,
  type ChatMessage,
} from '../lib/store';
import { BrandMark, OpenLockIcon, PaperclipIcon, PeopleIcon, SendIcon } from './icons';

interface Props {
  conversation: ChatConversation | null;
  messages: ChatMessage[];
  meId: string;
  meHandle?: string;
  loading: boolean;
  sending: boolean;
  error: string | null;
  onSend: (text: string) => Promise<void>;
  onSendAttachments: (
    files: File[],
    onProgress?: (loaded: number, total: number) => void,
  ) => Promise<void>;
  onDownload: (ref: AttachmentRef) => Promise<void>;
  fetchAttachmentBytes: (ref: AttachmentRef) => Promise<Blob>;
  title?: string | null;
  isActivePane?: boolean;
  headerActions?: ReactNode;
  /** Read marker when this conversation opened; later messages are new. */
  unreadAfterSeq?: number | null;
  /** Whether the end of the history is on screen; reported synchronously. */
  onTailVisibleChange?: (visible: boolean) => void;
  /** Names of other people typing here right now. */
  typingNames?: string[];
  /** The draft changed and is not empty (drives "is typing" for others). */
  onDraftActivity?: () => void;
  /** Workspace member names for channel authors (channels carry no roster). */
  directory?: Readonly<Record<string, MemberProfileBody>>;
}

function typingText(names: string[]): ReactNode {
  if (names.length === 0) return null;
  if (names.length === 1) return <><strong>{names[0]}</strong> is typing…</>;
  if (names.length === 2) return <><strong>{names[0]}</strong> and <strong>{names[1]}</strong> are typing…</>;
  return 'Several people are typing…';
}

// Within this distance of the end, the reader counts as at the tail.
const TAIL_SLACK_PX = 48;
// Breathing room left above the NEW divider when opening at it.
const DIVIDER_MARGIN_PX = 16;

// Same author within five minutes reads as one block: no repeated header.
const GROUP_WINDOW_MS = 5 * 60 * 1000;
function continuesGroup(prev: ChatMessage | undefined, next: ChatMessage): boolean {
  if (!prev || prev.sender_id !== next.sender_id || !prev.sent_at || !next.sent_at) return false;
  if (dayKey(prev.sent_at) !== dayKey(next.sent_at)) return false;
  const gap = new Date(next.sent_at).getTime() - new Date(prev.sent_at).getTime();
  return gap >= 0 && gap <= GROUP_WINDOW_MS;
}

export default function ConversationView({
  conversation,
  messages,
  meId,
  meHandle,
  loading,
  sending,
  error,
  onSend,
  onSendAttachments,
  onDownload,
  fetchAttachmentBytes,
  title,
  isActivePane = true,
  headerActions,
  unreadAfterSeq = null,
  onTailVisibleChange,
  typingNames = [],
  onDraftActivity,
  directory,
}: Props) {
  const [draft, setDraft] = useState('');
  const draftRevision = useRef(0);
  const [sendError, setSendError] = useState<string | null>(null);
  const [staged, setStaged] = useState<File[]>([]);
  const [uploading, setUploading] = useState(false);
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

  // The first message from someone else after the opening marker.
  const firstNewId = unreadAfterSeq == null
    ? null
    : messages.find(m => m.seq > unreadAfterSeq && m.sender_id !== meId)?.id ?? null;

  // Hidden mounted panes have no scroll box. Reconcile pending follows when
  // navigation or CSS breakpoint layout reveals this conversation, not on focus.
  const pendingTail = useRef(false);
  // Opening with an unread backlog lands on the NEW divider, not past it;
  // after that, only a reader already at the end is carried to new messages.
  const dividerPlaced = useRef<string | null>(null); // id of the placed divider's message
  const stickToTail = useRef(true);
  useLayoutEffect(() => {
    stickToTail.current = true;
    dividerPlaced.current = null;
  }, [conversationId]);
  useLayoutEffect(() => {
    pendingTail.current = true;
  }, [messages.length, conversationId]);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atTail = () => el.scrollHeight - el.scrollTop - el.clientHeight <= TAIL_SLACK_PX;
    const followPendingTail = () => {
      if (pendingTail.current && el.clientHeight > 0) {
        // Each distinct divider (on open, or on return to a hidden pane) is
        // positioned once; afterwards only a reader at the end is carried.
        const divider = firstNewId && dividerPlaced.current !== firstNewId
          ? el.querySelector<HTMLElement>('.new-divider')
          : null;
        if (divider) {
          el.scrollTop += divider.getBoundingClientRect().top - el.getBoundingClientRect().top - DIVIDER_MARGIN_PX;
          dividerPlaced.current = firstNewId;
          stickToTail.current = atTail();
        } else if (dividerPlaced.current === null || stickToTail.current) {
          el.scrollTop = el.scrollHeight;
        }
        pendingTail.current = false;
      }
      onTailVisibleChange?.(atTail());
    };
    const onScroll = () => {
      stickToTail.current = atTail();
      onTailVisibleChange?.(stickToTail.current);
    };
    followPendingTail();
    el.addEventListener('scroll', onScroll, { passive: true });
    // ResizeObserver also fires when display:none becomes a real layout box.
    // Consumed pending state leaves readers alone on ordinary viewport resizes.
    if (typeof ResizeObserver === 'undefined') return () => el.removeEventListener('scroll', onScroll);
    const observer = new ResizeObserver(followPendingTail);
    observer.observe(el);
    return () => {
      observer.disconnect();
      el.removeEventListener('scroll', onScroll);
    };
  }, [messages.length, conversationId, isActivePane, firstNewId, onTailVisibleChange]);

  async function submit(e: React.FormEvent): Promise<void> {
    e.preventDefault();
    if (sending || uploading || (!draft.trim() && staged.length === 0)) return;
    const revision = draftRevision.current;
    setSendError(null);
    try {
      if (staged.length > 0) {
        // Keep the selection mounted through the whole upload: the progress
        // row renders from `staged`, and a failure must leave the files
        // staged so retry doesn't force re-picking everything.
        const files = staged;
        setUploading(true);
        // Indeterminate until the transport reports bytes (the shell's
        // plugin fetch reports only per finished file).
        setUploadProgress(null);
        try {
          await onSendAttachments(files, (loaded, total) =>
            setUploadProgress(total > 0 ? loaded / total : 0),
          );
        } catch (err) {
          setSendError(err instanceof Error ? err.message : 'send failed');
          return;
        } finally {
          setUploading(false);
          setUploadProgress(null);
        }
        setStaged([]);
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
    const picked = Array.from(list).slice(0, MAX_ATTACHMENTS_PER_MESSAGE - staged.length);
    const oversize = picked.filter((f) => f.size > MAX_ATTACHMENT_BYTES);
    const unsupported = picked.filter((f) => !isAllowedAttachment(f));
    if (oversize.length > 0) {
      setSendError(
        `${oversize[0].name} exceeds 10 MiB and was not staged`,
      );
    } else if (unsupported.length > 0) {
      setSendError(`${unsupported[0].name} is not a supported file type and was not staged`);
    }
    setStaged((prev) =>
      [...prev, ...picked.filter((f) => f.size <= MAX_ATTACHMENT_BYTES && isAllowedAttachment(f))]
        .slice(0, MAX_ATTACHMENTS_PER_MESSAGE),
    );
    if (fileInput.current) fileInput.current.value = '';
  }

  if (!conversation) {
    return (
      <>
      {headerActions}
      <div className="conversation-empty">
        <span className="empty-mark" aria-hidden><BrandMark size={32} /></span>
        <span className="label-mono">A little closer, wherever you are</span>
        <h2>Your conversations, in one place</h2>
        <p>Select a conversation or open a DM to start chatting.</p>
      </div>
      </>
    );
  }

  const heading = title ?? conversationLabel(conversation, meId);
  const sub = title ? conversation.kind : conversationSublabel(conversation);
  const isChannel = conversation.kind === 'channel';
  const isGroup = conversation.kind === 'group';
  // One colour per person across the list, header and history: key by handle.
  const personKey = (senderId: string) => senderId === meId
    ? (meHandle ?? meId)
    : memberProfile(conversation, senderId)?.handle
      ?? (conversation.kind === 'dm' && conversation.peer_handle ? conversation.peer_handle : senderId);
  const peerKey = conversation.peer_handle ?? conversation.id;

  let lastDay = '';
  return (
    <div className="conversation-view flex min-h-0 flex-1 flex-col">
      <div className="conversation-header">
        <span className="conversation-title">
          {isGroup && <span aria-hidden className="tile h-7 w-7"><PeopleIcon size={15} /></span>}
          {!isChannel && !isGroup && <span aria-hidden className="avatar avatar-28" style={{ background: avatarGradient(peerKey) }}>{avatarInitial(conversation)}</span>}
          <span className="conversation-title-text">
            <h2>
              {/* Channel titles keep their literal "#name" text; the hash is styled as the glyph. */}
              {isChannel && heading.startsWith('#') ? <><span className="channel-glyph">#</span>{heading.slice(1)}</> : heading}
            </h2>
            {sub && (
              <span className="meta-mono truncate">
                {sub}
              </span>
            )}
          </span>
        </span>
        {headerActions}
      </div>
      <div className="trust-strip">
        <OpenLockIcon />
        <p className="plaintext-warning">
          Alpha demo: envelopes carry demo plaintext — not end-to-end encrypted.
        </p>
      </div>
      <div ref={scrollRef} className="message-history" aria-label="Message history">
        <div className="conversation-intro">
          {isChannel
            ? <span className="tile h-10 w-10 font-display text-xl" aria-hidden>#</span>
            : isGroup
              ? <span className="tile h-10 w-10" aria-hidden><PeopleIcon size={20} /></span>
              : <span className="avatar h-10 w-10 text-[15px]" aria-hidden style={{ background: avatarGradient(peerKey) }}>{avatarInitial(conversation)}</span>}
          <h3>{isChannel ? `Welcome to ${heading}` : isGroup ? `Your group with ${heading}` : `Your conversation with ${heading}`}</h3>
          <p>{isChannel ? 'A shared space for this workspace.' : isGroup ? 'Everyone here sees every message in this group.' : 'Your direct messages, together in one place.'}</p>
        </div>
        {loading && <p className="history-note meta-mono">Loading history…</p>}
        {messages.map((m, index) => {
          const divider =
            dayKey(m.sent_at) !== lastDay ? dayLabel(m.sent_at) : null;
          lastDay = dayKey(m.sent_at);
          const mine = m.sender_id === meId;
          const content = parseChatContent(m.ciphertext_b64);
          const author = senderLabel(meId, m.sender_id, conversation, directory);
          const isFirstNew = m.id === firstNewId;
          const follow = !divider && !isFirstNew && continuesGroup(messages[index - 1], m);
          const clock = formatClockTime(m.sent_at);
          return (
            <div key={m.id}>
              {divider && (
                <p className="day-divider">
                  <span>{divider}</span>
                </p>
              )}
              {isFirstNew && (
                <div className="new-divider" role="separator" aria-label="New messages">
                  <span aria-hidden>NEW</span>
                </div>
              )}
              <div className={`message-row${mine ? ' message-row-own' : ''}${follow ? ' message-row--follow' : ''}`}>
                <div className="message-gutter">
                  {follow
                    ? <span className="message-gutter-time font-mono" aria-hidden>{clock}</span>
                    : <span className="avatar avatar-32" aria-hidden style={{ background: avatarGradient(personKey(m.sender_id)) }}>
                      {mine ? (meHandle?.slice(0, 1).toUpperCase() || 'Y') : author.replace(/^@/, '').slice(0, 1).toUpperCase()}
                    </span>}
                </div>
                <div className="message-bubble">
                  {/* Follow-ups keep author, time and sequence for assistive tech and tooling. */}
                  <p className={follow ? 'message-meta sr-only' : 'message-meta'}>
                    <strong>{author}</strong>
                    <time dateTime={m.sent_at ?? undefined}>{clock}</time>
                    <span className="message-sequence">#{m.seq}</span>
                  </p>
                  {content.text && (
                    <p className="message-text whitespace-pre-wrap break-words [overflow-wrap:anywhere]">
                      {content.text}
                    </p>
                  )}
                  {content.attachments.map((attachment) => (
                    <AttachmentRow
                      key={attachment.id}
                      attachment={attachment}
                      onDownload={() => onDownload(attachment)}
                      fetchBytes={() => fetchAttachmentBytes(attachment)}
                    />
                  ))}
                </div>
              </div>
            </div>
          );
        })}
        {!loading && messages.length === 0 && (
          <p className="history-note">
            No messages yet — say bro.
          </p>
        )}
      </div>
      {error && <p role="alert" className="stage-alert text-alert">{error}</p>}
      {sendError && <p role="alert" className="stage-alert text-alert">{sendError}</p>}
      <form onSubmit={submit} className="message-composer">
        <div className="typing-indicator" aria-live="polite">
          {typingNames.length > 0 && <>
            <span className="typing-dots" aria-hidden><span /><span /><span /></span>
            <span className="truncate">{typingText(typingNames)}</span>
          </>}
        </div>
        <input
          ref={fileInput}
          type="file"
          multiple
          hidden
          aria-label="Attach files"
          accept="image/*,audio/*,video/*,.pdf,.zip"
          onChange={(e) => stageFiles(e.target.files)}
        />
        <div className="composer-field">
          {staged.length > 0 && (
            <div className="staged-files" aria-label="Staged files">
              {staged.map((file, index) => (
                <p key={`${file.name}-${index}`} className="staged-file">
                  <PaperclipIcon size={13} />
                  <span className="truncate">{file.name}</span>
                  <span className="meta-mono">{formatBytes(file.size)}</span>
                  <button
                    type="button"
                    className="btn btn-ghost btn-icon"
                    aria-label={`Remove ${file.name}`}
                    disabled={uploading}
                    onClick={() => setStaged((prev) => prev.filter((_, i) => i !== index))}
                  >
                    ✕
                  </button>
                </p>
              ))}
              {uploading && (
                <div className="upload-progress" role="progressbar" aria-label="Upload progress">
                  <div
                    className={uploadProgress === null ? 'upload-progress-indeterminate' : undefined}
                    style={{ width: `${Math.round((uploadProgress ?? 1) * 100)}%` }}
                  />
                </div>
              )}
            </div>
          )}
          <input
            aria-label="Message"
            placeholder="Message (demo plaintext → opaque envelope)"
            value={draft}
            onChange={(e) => {
              draftRevision.current += 1;
              setDraft(e.target.value);
              if (e.target.value.trim()) onDraftActivity?.();
            }}
          />
          <div className="composer-toolbar">
            <button
              type="button"
              className="btn btn-ghost btn-icon attach-button"
              aria-label="Attach files"
              title="Attach files (images, GIFs, audio, video, PDF, ZIP; max 10 MiB each)"
              disabled={sending || uploading || staged.length >= MAX_ATTACHMENTS_PER_MESSAGE}
              onClick={() => fileInput.current?.click()}
            >
              <PaperclipIcon size={15} />
            </button>
            <span className="composer-trust"><OpenLockIcon size={13} /><span className="truncate">Alpha demo · plaintext messages</span></span>
            <span className="composer-hint"><kbd className="kbd">Enter</kbd> to send</span>
            <button
              type="submit"
              disabled={sending || uploading || (!draft.trim() && staged.length === 0)}
              className="send-button"
            >
              {sending ? '…' : <><SendIcon size={15} /><span className="sr-only">Send</span></>}
            </button>
          </div>
        </div>
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
 * One attachment inside a message bubble. Images render an inline preview
 * (fetched on click to keep history scrolls cheap); other files render a
 * download row. The preview's object URL is revoked when it is replaced or
 * the row unmounts, and a fetch that resolves after unmount is revoked at
 * once instead of leaking up to 10 MiB per abandoned preview.
 */
function AttachmentRow({
  attachment,
  onDownload,
  fetchBytes,
}: {
  attachment: AttachmentRef;
  onDownload: () => Promise<void>;
  fetchBytes: () => Promise<Blob>;
}) {
  const [thumb, setThumb] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [loadingPreview, setLoadingPreview] = useState(false);
  const mounted = useRef(true);
  const isImage = attachment.mime.toLowerCase().startsWith('image/');

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

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
      const url = URL.createObjectURL(blob);
      if (!mounted.current) {
        URL.revokeObjectURL(url);
        return;
      }
      setThumb(url);
    } catch {
      if (mounted.current) setFailed(true);
    } finally {
      if (mounted.current) setLoadingPreview(false);
    }
  }

  // A rejected download (access revoked, blob missing, network) surfaces
  // on the row instead of becoming an unhandled rejection.
  async function download(): Promise<void> {
    setDownloadError(null);
    try {
      await onDownload();
    } catch (err) {
      if (mounted.current) {
        setDownloadError(err instanceof Error ? err.message : 'download failed');
      }
    }
  }

  const control = renderControl();
  return (
    <>
      {control}
      {downloadError && (
        <p role="alert" className="attachment-error">
          {`Could not download ${attachment.filename}: ${downloadError}`}
        </p>
      )}
    </>
  );

  function renderControl() {
    const size = <span className="meta-mono">{formatBytes(attachment.size_bytes)}</span>;
    if (!isImage) {
      return (
        <button type="button" onClick={() => void download()} className="attachment-row">
          <PaperclipIcon size={14} />
          <span className="truncate">{attachment.filename}</span>
          {size}
        </button>
      );
    }
    if (thumb && !failed) {
      return (
        <button type="button" onClick={() => void download()} className="attachment-preview" title={`Download ${attachment.filename}`}>
          <img
            src={thumb}
            alt={attachment.filename}
            onError={() => {
              // Undecodable bytes: clearing `thumb` runs the revoke effect
              // now instead of holding the blob until the row unmounts.
              setThumb(null);
              setFailed(true);
            }}
          />
        </button>
      );
    }
    // A failed preview keeps the row actionable: the label promises a
    // download, so a click downloads instead of dead-ending on a disabled
    // control after a transient failure.
    if (failed) {
      return (
        <button type="button" onClick={() => void download()} className="attachment-row">
          <PaperclipIcon size={14} />
          <span className="truncate">{`${attachment.filename} (preview failed, click to download)`}</span>
          {size}
        </button>
      );
    }
    return (
      <button
        type="button"
        disabled={loadingPreview}
        onClick={() => void showPreview()}
        className="attachment-row"
      >
        <PaperclipIcon size={14} />
        <span className="truncate">
          {loadingPreview
            ? `${attachment.filename}: loading preview…`
            : `${attachment.filename}: show preview`}
        </span>
        {size}
      </button>
    );
  }
}
