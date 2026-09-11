// Desktop native notifications for incoming DMs (issue #43).
//
// Design notes:
// - Focus detection: a DM arrival notifies ONLY when the main window is not
//   focused. `document.hasFocus()` is false when the window is backgrounded,
//   minimized, or another app is frontmost. No document (SSR) or a missing
//   API means "focused" -> silent, so an unknown state can never spam.
// - Permission is requested at most once per session (module flag); denial
//   is silent and never throws, so messaging always keeps working.
// - Tauri shell: official `@tauri-apps/plugin-notification` toast; click is
//   observed via `onAction` and focuses the main window.
// - Plain browsers: best-effort `window.Notification` with click-to-focus,
//   or silent when the API is absent. The plugin bridge is never touched
//   outside the shell, so the web build cannot break.
// - No secrets here: only the peer display name plus the demo plaintext the
//   UI already renders. No tokens, keys, or extra logging.

import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  isPermissionGranted,
  onAction,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification';
import { decodeOpaqueText, isTauriShell } from './api';
import {
  conversationLabel,
  truncatePreview,
  type ChatConversation,
  type ChatMessage,
} from './store';

export interface DmNotifyContent {
  title: string;
  body: string;
}

export type NotifyResult = 'sent' | 'skipped-focused' | 'skipped-denied';

let permissionRequested = false;
let clickWatched = false;

/** Test-only reset for the once-guards above. */
export function __resetNotificationStateForTests(): void {
  permissionRequested = false;
  clickWatched = false;
}

/**
 * Toast content for one incoming DM: peer name plus decoded text.
 * Null when there is nothing to toast (non-DM, own message, no row).
 */
export function buildDmNotification(
  meId: string,
  conv: ChatConversation | null,
  message: ChatMessage | null,
): DmNotifyContent | null {
  if (!conv || !message) return null;
  if (conv.kind !== 'dm') return null;
  if (message.sender_id === meId) return null;
  return {
    title: conversationLabel(conv),
    body: truncatePreview(decodeOpaqueText(message.ciphertext_b64), 120),
  };
}

/** True when the app window exists and does not hold OS focus. */
export function isWindowUnfocused(): boolean {
  try {
    if (typeof document === 'undefined') return false;
    if (typeof document.hasFocus !== 'function') return false;
    return !document.hasFocus();
  } catch {
    return false;
  }
}

/**
 * OS notification permission, requested at most once per app lifetime.
 * Deliberately not reset on logout: re-prompting after a denial is nagging,
 * and the OS settings path remains available. A reload re-arms the prompt.
 * Always resolves; denial or bridge failure means false, never a throw.
 */
export async function ensureNotificationPermission(): Promise<boolean> {
  try {
    if (isTauriShell()) {
      if (await isPermissionGranted()) return true;
      if (permissionRequested) return false;
      permissionRequested = true;
      return (await requestPermission()) === 'granted';
    }
    if (typeof window === 'undefined') return false;
    const Native = window.Notification;
    if (!Native) return false;
    if (Native.permission === 'granted') return true;
    if (Native.permission === 'denied') return false;
    if (permissionRequested) return false;
    permissionRequested = true;
    return (await Native.requestPermission()) === 'granted';
  } catch {
    return false;
  }
}

function showBrowserNotification(content: DmNotifyContent): void {
  const note = new window.Notification(content.title, {
    body: content.body,
  });
  note.onclick = () => {
    try {
      window.focus();
    } catch {
      // Focusing is best-effort; the toast already delivered.
    }
  };
}

/**
 * Toast one DM unless the window is focused or permission is denied.
 * Never throws: every failure degrades to a `skipped-*` result.
 */
export async function notifyDm(
  content: DmNotifyContent,
  opts?: { unfocused?: boolean },
): Promise<NotifyResult> {
  try {
    const unfocused = opts?.unfocused ?? isWindowUnfocused();
    if (!unfocused) return 'skipped-focused';
    if (!(await ensureNotificationPermission())) return 'skipped-denied';
    if (isTauriShell()) {
      try {
        await sendNotification({ title: content.title, body: content.body });
      } catch {
        return 'skipped-denied';
      }
      return 'sent';
    }
    showBrowserNotification(content);
    return 'sent';
  } catch {
    return 'skipped-denied';
  }
}

/** Bring the main window frontmost; best-effort, never throws. */
export async function focusMainWindow(): Promise<void> {
  try {
    if (isTauriShell()) {
      try {
        await getCurrentWindow().setFocus();
        return;
      } catch {
        // Fall through to window.focus() below.
      }
    }
    if (typeof window !== 'undefined') window.focus();
  } catch {
    // Focusing must never break messaging.
  }
}

/**
 * Observe toast clicks once per session and focus the app window.
 * No-op in plain browsers (each toast wires its own onclick).
 */
export async function watchNotificationClick(): Promise<void> {
  if (clickWatched) return;
  clickWatched = true;
  try {
    if (!isTauriShell()) return;
    await onAction(() => {
      void focusMainWindow();
    });
  } catch {
    // The click listener is best-effort; toasts still deliver.
  }
}
