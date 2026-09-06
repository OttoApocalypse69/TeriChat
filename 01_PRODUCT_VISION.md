# Product Vision

## 1. What the product is

The project is a secure real-time communications and collaboration platform.

It should be able to present itself differently depending on the workspace:

- a private messenger;
- a Discord-like community;
- a Slack-like team workspace;
- a study group;
- a virtual office;
- a cinema/watch-party space;
- a project coordination suite;
- a bot/plugin platform;
- an AI-assisted work environment.

The underlying architecture should remain shared.

## 2. Product philosophy

Do not force a false choice between "consumer social app" and "professional collaboration suite."

A workspace may contain any combination of:

```text
Communications
├── Text channels
├── DMs
├── Group DMs
├── Threads
├── Voice
├── Video
└── Broadcast/Cinema rooms

Collaboration
├── Whiteboards
├── Documents
├── Tasks
├── Projects
├── Calendars
└── Meetings

Social
├── Profiles
├── Stats
├── Achievements
└── Economy

Media
└── SPICE-powered Music

Platform
├── Bots
├── Webhooks
├── Slash commands
├── OAuth/application installs
└── WebAssembly plugins

Intelligence
└── AI runtime and agents
```

## 3. Workspace templates

Templates are presets, not separate architectures.

Suggested templates:

### Community

- text channels;
- voice channels;
- cinema/watch rooms;
- Stats;
- Music;
- Economy;
- bots/plugins;
- roles and moderation.

### Team

- channels;
- threads;
- voice/video;
- tasks;
- whiteboards;
- documents;
- meetings;
- integrations;
- AI assistant.

### Company

- everything from Team;
- stronger audit/admin controls;
- retention policies;
- workspace AI policies;
- SSO later;
- professional subscription/entitlement features.

### Study

- chat;
- group voice;
- whiteboards;
- shared documents;
- notes;
- task lists;
- AI study assistant;
- cinema/presentation room for lectures.

### Custom

Workspace owner chooses modules manually.

## 4. Global accounts, independent workspaces

Accounts are global.

Workspaces are independently owned and governed.

A user can belong to many workspaces. The platform operator is not implicitly a member of user-created workspaces.

Workspace administration must be scoped to that workspace only.

## 5. DMs and group DMs

Do not model DMs as hidden workspaces.

They are distinct social objects.

### Direct Message

- two participants;
- multi-device;
- E2EE;
- lightweight metadata.

### Group DM

- 3+ participants;
- E2EE;
- member management;
- optional group calls;
- no full role/channel hierarchy unless explicitly promoted to a workspace.

## 6. First-class channel/object types

Potential first-class channel/object kinds:

```text
TEXT
VOICE
CINEMA
BROADCAST
WHITEBOARD
DOCUMENT
TASK_BOARD
FORUM
ANNOUNCEMENT
STAGE
```

Do not implement every type in Alpha.

The architecture should permit typed channels/modules without abusing a giant generic JSON blob.

## 7. Consumer/community experience

Expected consumer features include:

- profiles;
- avatars and animated avatars;
- themes and cosmetics;
- servers/workspaces;
- channels;
- DMs;
- group chats;
- reactions;
- custom emoji/stickers;
- voice/video;
- screen sharing;
- Music;
- Stats;
- achievements;
- economy;
- bots/plugins;
- cinema nights;
- spatial audio;
- AI assistance.

## 8. Professional experience

Expected professional features include:

- workspaces;
- role-based access;
- project channels;
- threads;
- searchable documents where policy permits;
- whiteboards;
- task/project tracking;
- meeting notes;
- meeting transcription when explicitly enabled;
- action item extraction;
- GitHub and other integrations;
- audit logs;
- retention;
- professional billing;
- AI agents;
- later SSO/SAML/OIDC.

## 9. Product identity still unresolved

The product name **TeriChat** is temporary.

Reason: the platform has grown beyond a personal chat application.

The security brand **TeriCrypt-4096™** remains.

See `12_OPEN_DECISIONS.md`.
