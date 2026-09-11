// @vitest-environment jsdom
// Regression tests for issue #43: desktop native notifications for DMs.
// RED-first: this file is written before src/lib/notifications.ts exists.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const plugin = vi.hoisted(() => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
  sendNotification: vi.fn(),
  onAction: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-notification', () => plugin);

const winApi = vi.hoisted(() => ({ setFocus: vi.fn() }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ setFocus: winApi.setFocus }),
}));

import {
  __resetNotificationStateForTests,
  buildDmNotification,
  ensureNotificationPermission,
  isWindowUnfocused,
  notifyDm,
  watchNotificationClick,
} from './notifications';
import { encodeOpaqueText } from './api';
import type { ChatConversation, ChatMessage } from './store';

const dmConv = (over: Partial<ChatConversation> = {}): ChatConversation => ({
  id: 'dm',
  kind: 'dm',
  members: ['me', 'peer'],
  peer_handle: 'peer',
  peer_display_name: 'Peer',
  last_seq: null,
  last_sent_at: null,
  ...over,
});

const msg = (over: Partial<ChatMessage> = {}): ChatMessage => ({
  id: 'm1',
  conversation_id: 'dm',
  sender_id: 'peer',
  seq: 1,
  ciphertext_b64: encodeOpaqueText('hello bro'),
  client_msg_id: 'c1',
  sent_at: '2026-09-01T00:00:00Z',
  ...over,
});

function asShell() {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}
function asBrowser() {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}
function focusState(focused: boolean) {
  vi.spyOn(document, 'hasFocus').mockReturnValue(focused);
}

class FakeNotification {
  static permission = 'granted';
  static instances: FakeNotification[] = [];
  static requestPermission = vi.fn(async () => FakeNotification.permission);
  onclick: (() => void) | null = null;
  title: string;
  opts: unknown;
  constructor(title: string, opts?: unknown) {
    this.title = title;
    this.opts = opts;
    FakeNotification.instances.push(this);
  }
}

beforeEach(() => {
  vi.restoreAllMocks();
  asShell();
  plugin.isPermissionGranted.mockReset().mockResolvedValue(true);
  plugin.requestPermission.mockReset().mockResolvedValue('granted');
  plugin.sendNotification.mockReset();
  plugin.onAction.mockReset().mockResolvedValue(undefined);
  winApi.setFocus.mockReset().mockResolvedValue(undefined);
  FakeNotification.instances = [];
  FakeNotification.permission = 'granted';
  FakeNotification.requestPermission.mockClear();
  delete (window as unknown as Record<string, unknown>).Notification;
  __resetNotificationStateForTests();
});

describe('buildDmNotification', () => {
  it('toasts the peer name plus decoded text for a DM', () => {
    expect(buildDmNotification('me', dmConv(), msg())).toEqual({
      title: 'Peer',
      body: 'hello bro',
    });
  });

  it('stays silent for own messages and non-DM conversations', () => {
    expect(buildDmNotification('me', dmConv(), msg({ sender_id: 'me' }))).toBeNull();
    expect(
      buildDmNotification('me', dmConv({ kind: 'channel' }), msg()),
    ).toBeNull();
  });
});

describe('isWindowUnfocused', () => {
  it('mirrors document focus (unfocused arrival notifies, focused stays silent)', () => {
    focusState(false);
    expect(isWindowUnfocused()).toBe(true);
    focusState(true);
    expect(isWindowUnfocused()).toBe(false);
  });
});

describe('notifyDm in the Tauri shell', () => {
  it('sends exactly one toast per unfocused arrival', async () => {
    focusState(false);
    const result = await notifyDm({ title: 'Peer', body: 'hello bro' });
    expect(result).toBe('sent');
    expect(plugin.sendNotification).toHaveBeenCalledTimes(1);
    expect(plugin.sendNotification).toHaveBeenCalledWith({
      title: 'Peer',
      body: 'hello bro',
    });
  });

  it('stays silent when the window is focused', async () => {
    focusState(true);
    const result = await notifyDm({ title: 'Peer', body: 'hello bro' });
    expect(result).toBe('skipped-focused');
    expect(plugin.sendNotification).not.toHaveBeenCalled();
    expect(plugin.requestPermission).not.toHaveBeenCalled();
  });

  it('denied permission never throws and never re-asks', async () => {
    focusState(false);
    plugin.isPermissionGranted.mockResolvedValue(false);
    plugin.requestPermission.mockResolvedValue('denied');
    await expect(
      notifyDm({ title: 'Peer', body: 'hello bro' }),
    ).resolves.toBe('skipped-denied');
    await expect(
      notifyDm({ title: 'Peer', body: 'hello bro' }),
    ).resolves.toBe('skipped-denied');
    expect(plugin.requestPermission).toHaveBeenCalledTimes(1);
    expect(plugin.sendNotification).not.toHaveBeenCalled();
  });

  it('plugin failures degrade to silent denial instead of throwing', async () => {
    focusState(false);
    plugin.isPermissionGranted.mockRejectedValue(new Error('bridge down'));
    await expect(ensureNotificationPermission()).resolves.toBe(false);
    await expect(
      notifyDm({ title: 'Peer', body: 'hello bro' }),
    ).resolves.toBe('skipped-denied');
  });

  it('send rejection degrades to silent denial instead of an unhandled rejection', async () => {
    focusState(false);
    plugin.isPermissionGranted.mockResolvedValue(true);
    plugin.sendNotification.mockRejectedValue(new Error('toast failed'));
    await expect(
      notifyDm({ title: 'Peer', body: 'hello bro' }),
    ).resolves.toBe('skipped-denied');
  });
});

describe('notifyDm in plain browsers', () => {
  beforeEach(() => {
    asBrowser();
    (window as unknown as Record<string, unknown>).Notification =
      FakeNotification;
  });

  it('uses the best-effort Notification API when granted', async () => {
    focusState(false);
    const result = await notifyDm({ title: 'Peer', body: 'hello bro' });
    expect(result).toBe('sent');
    expect(FakeNotification.instances).toHaveLength(1);
    expect(FakeNotification.instances[0].title).toBe('Peer');
  });

  it('clicking the browser toast focuses the window without throwing', async () => {
    focusState(false);
    const focusSpy = vi.spyOn(window, 'focus').mockImplementation(() => {});
    await notifyDm({ title: 'Peer', body: 'hello bro' });
    expect(() =>
      FakeNotification.instances[0].onclick?.(),
    ).not.toThrow();
    expect(focusSpy).toHaveBeenCalled();
  });

  it('stays silent with zero breakage when the API is missing', async () => {
    focusState(false);
    delete (window as unknown as Record<string, unknown>).Notification;
    await expect(
      notifyDm({ title: 'Peer', body: 'hello bro' }),
    ).resolves.toBe('skipped-denied');
  });
});

describe('watchNotificationClick', () => {
  it('focuses the main window on toast click and registers only once', async () => {
    let clicked: (() => void) | null = null;
    plugin.onAction.mockImplementation(async (cb: () => void) => {
      clicked = cb;
      return () => {};
    });
    await watchNotificationClick();
    await watchNotificationClick();
    expect(plugin.onAction).toHaveBeenCalledTimes(1);
    expect(clicked).not.toBeNull();
    await (clicked as unknown as () => Promise<void>)();
    expect(winApi.setFocus).toHaveBeenCalledTimes(1);
  });

  it('never throws when the listener cannot register', async () => {
    plugin.onAction.mockRejectedValue(new Error('bridge down'));
    await expect(watchNotificationClick()).resolves.toBeUndefined();
  });
});
