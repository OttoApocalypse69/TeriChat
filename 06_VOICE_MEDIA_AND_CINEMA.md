# Voice, Media, Spatial Audio, and Cinema

## 1. Voice architecture

Use WebRTC concepts.

Expected infrastructure:

- STUN/TURN;
- coturn;
- SFU for group voice/video;
- separate participant streams;
- client-side mixing/spatialization.

Avoid full-mesh as the long-term architecture for larger groups.

## 2. Spatial audio

Spatial audio should be an optional voice-room mode.

Each participant can have:

```text
x
y
optional z
orientation
```

Client receives independent participant streams, decrypts them, and applies:

- stereo positioning;
- HRTF where supported;
- distance attenuation;
- optional proximity curves;
- room-specific acoustic effects later.

Server should not pre-mix everyone if spatialization is desired.

## 3. Virtual room layouts

A voice channel may optionally have a 2D room layout.

Examples:

- lounge;
- conference table;
- classroom;
- auditorium;
- cinema;
- virtual office.

Room objects can contain:

- participant positions;
- presentation screen positions;
- whiteboard anchors;
- Music source position;
- interaction zones.

This is not intended to require VR.

## 4. Music system

The built-in Music service is a first-party system service.

### SPICE integration

SPICE is the preferred media/source backend.

TeriChat should own:

- queue;
- room association;
- permissions;
- playback state;
- synchronization;
- voice injection;
- Stats events.

SPICE should own, where appropriate:

- search;
- metadata;
- provider/source resolution;
- media acquisition/playback source logic.

Do not duplicate provider-specific logic in TeriChat if SPICE already provides it.

### Music privacy

Music should ideally be able to send audio without receiving/decrypting user microphone audio.

### Always-on Music

Boosted/professional workspaces may unlock:

- persistent queues;
- continuous radio mode;
- scheduled playback;
- more concurrent Music instances;
- always-ready workers.

## 5. Cinema / Watch Room

Cinema is a first-class channel type.

It is optimized for synchronized passive viewing rather than ordinary VC conversation.

Logical planes:

```text
MEDIA PLANE
primary synchronized video/audio

AUDIENCE PLANE
viewers, reactions, presence

COMMUNICATION PLANE
comments, whispers, private side groups
```

## 6. Cinema media sources

Support only media the user/workspace is authorized to play/share.

Possible sources:

- user-provided/local media;
- authorized provider integrations;
- legitimate direct streams;
- ordinary screen share as fallback.

The platform should not implement DRM bypass or unauthorized media extraction.

## 7. Cinema playback synchronization

Cinema session state can contain:

```text
session_id
media_id
state
authoritative_position
playback_rate
epoch/timestamp
subtitle track
audio track
quality profile
```

Clients should:

- maintain a buffer;
- estimate drift;
- subtly correct small drift;
- resync larger drift;
- synchronize new joiners.

## 8. Cinema conversation modes

Potential room modes:

### Silent Cinema

Audience microphones disabled or non-broadcast by default.

### Commentary

Normal audience discussion allowed.

### Host Commentary

Only designated speakers/commentators broadcast room-wide.

### Intermission

Normal conversation temporarily enabled.

## 9. Whisper / point-to-point communication

Cinema participants should be able to communicate without disrupting the main audience.

Possible forms:

- private voice whisper;
- temporary voice subgroup;
- private text;
- friend-only comments.

Spatial mode can make whispers directional/local.

This is logically separate from the main movie audio.

## 10. Cinema reactions

Provide lightweight ephemeral reactions:

```text
😂 😭 💀 ❤️ 😱 🔥
```

Stats may record aggregate reaction events and interesting timestamps.

Avoid cluttering the movie screen.

## 11. Cinema quality and boost entitlements

Cinema is a good place to map paid infrastructure to real cost.

Possible quality ladder:

### Free

- 1080p target;
- standard bitrate;
- stereo.

### Higher workspace level

- higher bitrate 1080p;
- surround;
- 1440p;
- larger buffers;
- better codec profiles.

### Highest boost/pro tiers

- 4K where infrastructure and client capability permit;
- HDR later;
- high-bitrate surround;
- premium transcoding capacity.

Exact numbers remain an open decision.

## 12. Generalized Broadcast Room

Do not hard-code the underlying engine solely for movies.

A shared BroadcastRoom primitive can power:

- Cinema;
- Presentation;
- Lecture;
- Tournament Broadcast;
- Company All-Hands;
- Livestream.

Cinema gets a specialized UI and policies on top.

## 13. AI in Cinema

Potential future AI functions:

- captions;
- translation;
- accessibility descriptions;
- scheduling;
- recap;
- character/context questions.

For spoiler-aware questions, AI should be constrained to information known before the current playback timestamp when feasible.
