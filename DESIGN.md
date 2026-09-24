# UnknownChat — Obsidian Violet

Implementation reference adapted from the owner's Google Stitch export,
`stitch_unknownchat_canonical_design_system`, supplied September 24, 2026.
The four supplied screens cover direct messages, workspace text channels with
details closed, login, and security/device settings. This document records the
visual direction, not a claim that illustrative features exist.

## Foundation

Dark charcoal surfaces, restrained violet selection and primary actions,
thin borders, and readable message rows. No remote fonts, CDN scripts, photos,
or inline handlers from the exported HTML are required by the application.

| Token | Value | Use |
|---|---|---|
| Canvas | `#0A0B10` | Chat and page background |
| Rail | `#0E1017` | Workspace switcher and app header |
| Sidebar | `#131620` | Navigation, composer, own message rows |
| Surface | `#1A1D28` | Login and settings cards |
| Elevated | `#222634` | Buttons and avatars |
| Hover | `#2B3042` | Hover feedback |
| Subtle border | `#252B3B` | Surface separation |
| Strong border | `#384157` | Input boundaries |
| Primary text | `#F1F5F9` | Titles and important content |
| Secondary text | `#94A3B8` | Supporting text |
| Violet | `#8A4FFF` | Primary actions and selected rail icons |
| Violet highlight | `#9D6BFF` | Focus rings |
| Action hover | `#7740DE` | Maintain white button-text contrast on hover |
| Soft violet | `rgba(138,79,255,.14)` | Selected navigation and own avatar |

Inter is the preferred UI font with system sans-serif fallbacks. Identifiers
use a local monospace stack. No network font request is made. Message text is
14px with generous line spacing; supporting copy is 12–13px. Uppercase section
labels and timestamps are compact. Shapes use 6–12px corners, with circular
account avatars and no large decorative gradients.

## Layout and interaction

- Desktop: a 64px workspace rail, 280px conversation sidebar, flexible chat
  area with a 900px maximum reading width, and optional 320px details panel.
- Workspace details start closed, with explicit open and back controls.
- Below 1200px, details replace the chat pane. Below 768px, navigation, chat,
  and details show one at a time. CSS visibility keeps drafts and pending
  operations mounted. Selecting a different conversation resets its draft.
- Navigation filtering matches loaded channel names and conversation labels;
  it does not search message content or issue backend queries.
- Sign-in uses a centered 420px card. The sessions view replaces the chat
  visually while keeping it mounted, and uses ordered, responsive cards
  with actual IDs, expiry and creation times, and the current-session marker.
- Inputs have visible focus rings; compact mobile actions have 44px targets.
  Reduced-motion preferences disable the existing connection pulse.

## Truthful product states

The existing Alpha 0 transport carries demo plaintext envelopes. Keep that
disclosure visible. Gateway connection status reflects the existing client.
The design's encryption, P2P, fingerprint, passkey, MFA, device-location,
attachment, voice and media examples are not implemented by this visual work.
Session revocation is distinct from removing cryptographic device membership.

API, authentication, permissions, crypto and pagination behavior remain under
their existing contracts. The imported styling does not change those contracts.

## Verification

Run `npm test`, `npm run typecheck`, and `npm run build` in `apps/desktop`.
`node acceptance/stitch-ui.mjs` checks responsive UI against synthetic HTTP
fixtures and writes timestamped local screenshots/results under `.acceptance`.
These fixtures are not backend evidence. The unchanged real-API acceptance
workflow separately exercises the application against PostgreSQL and Axum.
