# UnknownChat — design language

This is the implementation reference for the shared React client in
`apps/desktop` (Tauri app and web build). It describes what ships. Tokens are
defined once in `apps/desktop/src/index.css` and mirrored in
`apps/desktop/tailwind.config.js`. Build new UI from those tokens, never from
raw hex values.

The language was adopted in `062ab2d` (2026-09-25). It came from the owner's
design canvas, whose Foundations, Components, Main, DMs and Security boards are
not stored in this repository. It replaced the Obsidian Violet direction
(Google Stitch export, 2026-09-24). See *History* below.

## Foundations

Dark surfaces get darker toward the edges and lighter at the center. The only
brand hue is "ultraviolet", used as light rather than as fill. Thin lines
separate zones. Spaces, channels and built-ins use cut-corner tiles; people use
circles.

### Surfaces and lines

| Token | Value | Use |
|---|---|---|
| `--bg-chassis` | `#07070a` | Top bar, rail, system bar, page |
| `--bg-panel` | `#0b0b10` | Navigator and context panel |
| `--bg-stage` | `#0f0f15` | Conversation stage |
| `--bg-raised` | `#14141b` | Row hover |
| `--bg-field` | `#121219` | Inputs and composer |
| `--bg-overlay` | `#1a1a23` | Secondary buttons |
| `--bg-selected` | `#1a1a24` | Current row |
| `--bg-card` | `#111118` | Cards and panel rows |
| `--line-subtle` / `default` / `strong` / `heavy` | `#1b1b24` / `#262632` / `#2e2e3e` / `#34344a` | Zone, card, control and emphasis borders |

### Ultraviolet

`--uv-100` `#e2d6ff` through `--uv-700` `#6440db`. `--uv-500` `#9a78ff` is
for focus rings, selected glyphs and the unread pip. `--uv-600` `#7b52f4` is
for primary buttons, send and unread badges. Washes (`--uv-wash-07` to `-22`)
mark selected or accent states.

### Ink

Ink is measured on `--bg-stage`. `--ink-1` `#eceaf4` is for titles,
`--ink-body` `#d9d6e4` for message text, `--ink-2` `#b4b1c4` for supporting
text, and `--ink-3` `#8a879e` for meta text. `--ink-3` is the lightest ink any
readable text may use. `--ink-off` is for disabled controls only.

### Signals

`--mint` (connected, success), `--amber` (warning, plaintext disclosure),
`--rose` (danger, errors) and `--sky` (info). A signal color always comes with
an icon, a shape or a word, never color alone.

### Type

All type is self-hosted through `@fontsource` (OFL-1.1). The app never
contacts a font CDN.

- Display: **Chakra Petch** 500–700 (`.display-m`, empty states, intros).
- UI: **Instrument Sans** 400–700. Body is 13/18. Message text is 14/21 with a
  72ch maximum width.
- Mono: **JetBrains Mono** (`.label-mono`, `.meta-mono`, timestamps, ids,
  tags, the system bar).

### Shape, elevation and motion

Radii are `--r-xs` 3, `--r-sm` 5, `--r-md` 8 and `--r-lg` 12. Chamfers
(`--chamfer-6/7/8`) cut the rail tiles and the brand tile. Easing is
`--ease`, with durations `--t-instant` 90ms, `--t-quick` 140ms and `--t-panel`
200ms. `prefers-reduced-motion` turns off transitions, the reconnect pulse and
the typing dots.

## Components

- **Buttons:** `.btn` plus one of `-primary` (at most one per view),
  `-secondary`, `-ghost`, `-accent`, `-danger` or `-icon`. Sizes are `-sm`,
  default and `-lg`.
- **Fields:** `.field` (32px), `-sm` and `-mono`. The label goes above and
  helper text below.
- **Tags:** mono, uppercase, with a 1px ring at 35% (`.tag-uv`, `-sky`,
  `-amber`, `-mint`, `-rose`, `-line`, `-fill`). Used for roles and phase.
- **Avatars:** circles with a stable per-person gradient (`lib/avatar.ts`), in
  sizes 18, 24, 28 and 32.
- **Tiles:** chamfered gradient tiles for workspaces, channels and groups.
- **Rows:** `.nav-row` (30px) and `.conversation-row` (name, time and preview).
  The current row has `aria-current="page"`. Unread rows get a uv pip at the
  edge and a count badge.
- **Messages:** gutter-based rows. Messages from the same author within five
  minutes are grouped under one header. Sequence markers stay in the DOM for
  assistive tech and the acceptance tooling. A "NEW" divider sits at the read
  marker.

## Shell

Six zones:

| Zone | Size | Contents |
|---|---|---|
| Top bar | 44px tall | Brand, phase tag, Sessions, Log out, account |
| Rail | 64px wide | Conversations home, workspace tiles, version |
| Navigator | 248px wide | Heading, filter, channels, DMs and groups, create/join |
| Stage | flexible | Header, trust strip, history, typing, composer |
| Context panel | 340px wide, starts closed | Members, activity, invites |
| System bar | 32px tall | Gateway status, trust badge |

Breakpoints:

- Below 1200px the context panel replaces the stage.
- Below 768px the rail and navigator, the stage, and the context panel show one
  at a time, and every control is at least 44px tall.
- CSS visibility keeps hidden panes mounted, so drafts and pending operations
  survive.
- Sessions replace the layout visually while it stays mounted.

## Truthful product states

The Alpha 0 transport carries demo plaintext envelopes. That disclosure must
stay visible. Draw no E2EE, verified-device, fingerprint, passkey, MFA,
presence, reaction, thread, voice, cinema or plugin UI until those features
exist. Gateway status reflects the real client connection. Revoking a session
is not the same as removing a cryptographic device.

API, authentication, permissions, crypto and pagination behavior stay under
their existing contracts. Styling never changes them.

## Verification

Run `npm test`, `npm run typecheck` and `npm run build` in `apps/desktop`.
`node acceptance/stitch-ui.mjs` checks the responsive UI against synthetic HTTP
fixtures and writes screenshots under `.acceptance`. Those fixtures are not
backend evidence. The real-API acceptance workflow separately exercises the
client against PostgreSQL and Axum.

## History

**Obsidian Violet** (2026-09-24, `326dafd`) was adapted from the owner's
Google Stitch export `stitch_unknownchat_canonical_design_system`. It used
Inter, a `#0A0B10` canvas, `#8A4FFF` violet, a 280px sidebar and a 320px
details panel. It was replaced by the language above the next day. Its layout
rules (mounted panes, closed details, one pane on phones, 44px targets) carried
over.
