// @vitest-environment jsdom
// Attachments v1: chat-content encode/parse round-trips plus client guards.
// Refs travel inside the existing envelope JSON; the server stores bytes.
import { describe, expect, it } from 'vitest';
import {
  encodeChatContent,
  encodeOpaqueText,
  MAX_ATTACHMENTS_PER_MESSAGE,
  MAX_ATTACHMENT_BYTES,
  parseChatContent,
  type AttachmentRef,
} from '../api';

const ref = (id: string): AttachmentRef => ({
  id,
  filename: 'photo.png',
  mime: 'image/png',
  size_bytes: 1234,
  sha256: 'synthetic-sha',
});

describe('encodeChatContent', () => {
  it('leaves plain-text messages as bare strings (legacy path)', () => {
    expect(encodeChatContent('bro', [])).toBe('bro');
  });

  it('embeds refs as JSON when attachments exist', () => {
    const encoded = encodeChatContent('look', [ref('a1')]);
    const parsed = JSON.parse(encoded) as { text: string; attachments: AttachmentRef[] };
    expect(parsed.text).toBe('look');
    expect(parsed.attachments).toHaveLength(1);
    expect(parsed.attachments[0].id).toBe('a1');
  });
});

describe('parseChatContent', () => {
  it('parses legacy bare-text envelopes', () => {
    const payload = encodeOpaqueText('bro');
    expect(parseChatContent(payload)).toEqual({ text: 'bro', attachments: [] });
  });

  it('round-trips text plus refs (incl. GIF mime)', () => {
    const gif: AttachmentRef = {
      id: 'g1',
      filename: 'dance.gif',
      mime: 'image/gif',
      size_bytes: 4242,
      sha256: 'synthetic-sha',
    };
    const payload = encodeOpaqueText(encodeChatContent('lol', [gif]));
    const content = parseChatContent(payload);
    expect(content.text).toBe('lol');
    expect(content.attachments).toHaveLength(1);
    expect(content.attachments[0].mime).toBe('image/gif');
  });

  it('drops malformed attachment entries instead of throwing', () => {
    const payload = encodeOpaqueText(
      JSON.stringify({ text: 'hi', attachments: [{ nope: true }] }),
    );
    expect(parseChatContent(payload)).toEqual({ text: 'hi', attachments: [] });
  });

  it('degrades non-envelope payloads to displayable text', () => {
    expect(parseChatContent('not-an-envelope').text).toBe('not-an-envelope');
  });
});

describe('attachment client guards', () => {
  it('caps uploads at 10 MiB', () => {
    expect(MAX_ATTACHMENT_BYTES).toBe(10 * 1024 * 1024);
  });

  it('caps refs per message at 5', () => {
    expect(MAX_ATTACHMENTS_PER_MESSAGE).toBe(5);
    const refs = Array.from({ length: 7 }, (_, i) => ref(`r${i}`));
    expect(refs.slice(0, MAX_ATTACHMENTS_PER_MESSAGE)).toHaveLength(5);
  });
});
