# Workspaces and Collaboration

## 1. Workspace model

A Workspace is an independently governed social/professional space.

Workspace fields should eventually include:

```text
id
name
icon
owner
privacy policy
invite policy
default role
enabled modules
encryption policy
retention policy
bot policy
plugin policy
AI policy
subscription/boost state
```

## 2. Roles and permissions

Potential roles:

- Owner
- Administrator
- Moderator
- Member
- Guest
- DJ
- Economy Manager
- Bot Manager
- Plugin Manager
- Project Manager

Potential permissions:

```text
MANAGE_WORKSPACE
MANAGE_CHANNELS
MANAGE_ROLES
MANAGE_MEMBERS
KICK_MEMBERS
BAN_MEMBERS

SEND_MESSAGES
MANAGE_MESSAGES

CONNECT_VOICE
MUTE_MEMBERS
MOVE_MEMBERS

MANAGE_MUSIC
MANAGE_ECONOMY
MANAGE_BOTS
MANAGE_PLUGINS

MANAGE_PROJECTS
MANAGE_TASKS
MANAGE_DOCUMENTS
MANAGE_WHITEBOARDS

MANAGE_AI
VIEW_AUDIT_LOG
```

## 3. Invitations

Invite objects can support:

- expiration;
- maximum uses;
- creator;
- approval requirement;
- initial role;
- revocation;
- workspace-specific policies.

## 4. Whiteboards

Whiteboards are first-class collaborative objects.

Do not model every mouse move as an isolated REST write.

Prefer a CRDT-based collaborative state model.

Potential operations:

```text
shape.create
shape.move
shape.resize
shape.delete

text.insert
text.delete

connector.create
cursor.move
selection.change
```

The collaboration substrate may later be reused by:

- documents;
- notes;
- task descriptions;
- meeting notes;
- canvas-like boards.

## 5. Documents

Long-term document features:

- rich text;
- multiplayer editing;
- comments;
- references to messages/tasks/files;
- workspace permissions;
- revision history;
- AI summarization where policy allows.

## 6. Tasks and projects

Task system should support:

- title;
- description;
- assignee(s);
- status;
- due date;
- labels;
- dependencies;
- workspace/project scope;
- comments;
- links to messages/documents/whiteboards.

Project views may include:

- list;
- kanban;
- timeline later.

## 7. Meetings

Meeting objects may connect:

```text
voice/video session
meeting chat
shared notes
whiteboard
screen share
transcript (optional)
AI summary (optional)
action items
```

AI-generated action items should become real Task objects only through an explicit action/permission path.

## 8. Professional integrations

Future integrations may include:

- GitHub;
- calendars;
- cloud drives;
- issue trackers;
- CI/CD systems;
- webhooks;
- custom bots.

The platform's public bot/app permission system should support these rather than creating one-off privileged integrations.

## 9. Auditability

Professional workspaces will likely require:

- membership changes;
- role changes;
- bot/plugin installs;
- AI actions;
- policy changes;
- task/project administration;
- retention changes.

Do not include private message plaintext in audit logs.
