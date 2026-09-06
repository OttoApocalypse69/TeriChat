# AI Platform

## 1. AI is a platform primitive

Do not implement AI as only one chatbot in one channel.

AI should operate over platform objects, subject to explicit permissions and privacy policy.

Potential inputs/tools:

- messages;
- threads;
- documents;
- whiteboards;
- tasks;
- projects;
- meeting artifacts;
- files;
- GitHub integrations;
- calendars;
- Stats;
- workspace metadata.

## 2. AI runtime

Create a provider-agnostic AI gateway/runtime.

Potential provider adapters:

- OpenAI;
- Anthropic;
- Gemini;
- OpenRouter-compatible services;
- local OpenAI-compatible servers;
- user/local models;
- Potato or future user-trained models.

The runtime should own:

- provider abstraction;
- model routing;
- context construction;
- permission checks;
- tool execution;
- quotas;
- billing/usage metering;
- audit events;
- safety/policy boundaries;
- caching where appropriate.

## 3. Model roles

A workspace may configure:

```text
Fast model
cheap routing/classification

General model
normal assistant

Reasoning model
complex analysis/planning

Local/private model
sensitive local processing
```

## 4. AI capabilities

Examples:

```text
messages.read
threads.read
documents.read
documents.write
whiteboards.read
tasks.read
tasks.create
tasks.update
calendar.read
calendar.create
github.read
stats.read
```

AI must not receive universal implicit access.

## 5. Action risk levels

Suggested action classes:

### Read-only

- summarize;
- search;
- explain;
- inspect allowed workspace state.

### Reversible/low-risk write

- create draft;
- create task;
- add note;
- propose board update.

### Higher-risk write

- invite member;
- change permissions;
- publish announcement;
- delete content;
- trigger external deployment.

Higher-risk actions should require stronger confirmation/policy.

## 6. "What happened while I was gone?"

A flagship professional AI use case.

AI can summarize:

- channel activity;
- merged PRs;
- changed tasks;
- blockers;
- decisions;
- action items.

Only from data it is authorized to access.

## 7. Meetings

When explicitly enabled:

```text
voice
 ↓
speech-to-text
 ↓
speaker attribution
 ↓
meeting context
 ↓
notes / decisions / action items
```

Potential outputs:

- transcript;
- summary;
- decisions;
- action items;
- follow-up tasks.

Transcription/AI processing must be visibly enabled and policy-aware.

## 8. Local AI

Local models are important for privacy.

Potential local features:

- message search;
- summarization;
- writing assistance;
- classification;
- translation;
- personal assistant.

Local AI is especially useful where TeriCrypt-protected plaintext should never leave the client.

## 9. AI and E2EE

See `03_SECURITY_TERICRYPT_4096.md`.

Never silently upload decrypted E2EE content to a remote provider.

## 10. AI billing

AI can become a professional/premium entitlement because it has real variable cost.

Potential limits:

- monthly AI credits;
- model classes;
- workspace pooled credits;
- BYOK/provider connection later;
- local model option;
- metered enterprise usage.

Exact commercial model is open.
