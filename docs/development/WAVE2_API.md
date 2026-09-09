# Backend wave 2 API contracts

All HTTP endpoints require the existing bearer session. These additions use
existing tables, permissions, and counters; no migration is required.

| Method and path | Response and authority |
|---|---|
| `GET /v1/auth/sessions` | Caller-owned live sessions: `sessions` containing `id`, `device_id`, `created_at`, `expires_at`, `is_current`, plus `next_cursor`. No token or token hash. |
| `DELETE /v1/auth/sessions/{id}` | 204 for an owned session, including one already expired/revoked. Foreign and unknown IDs both return 404. Self-revocation is allowed; a revoked caller cannot authorize another request. |
| `GET /v1/workspaces/{id}/stats/me` | Caller `user_id`, `workspace_id`, `message_count`, and nullable `last_message_at`, aggregated across current channels. Requires current unbanned membership. |
| `GET /v1/workspaces/{id}/stats/me/channels` | `channels` with `channel_id`, `conversation_id`, `name`, caller-only `message_count` and nullable `last_message_at`, plus `next_cursor`. Includes zero-count current channels. |
| `GET /v1/workspaces/{id}/bans` | `bans` with `user_id`, `handle`, `display_name`, `banned_by`, `reason`, `banned_at`, plus `next_cursor`. Requires existing `BanMembers` permission (owner/admin). |

Lists accept an exclusive UUID `after` cursor and positive integer `limit`.
Default and maximum page size are 100; larger positive values are capped.
Malformed values and nonpositive limits return 400. The cursor orders session,
channel, or banned-user IDs respectively. Pagination is a live view, not a
multi-request snapshot; concurrent changes may alter later pages.

Stats are eventually consistent outbox projections. DMs, other workspaces,
other users' counters, and message bodies are excluded. Deleted channels stop
contributing; a rejoined member can see their retained counters on current
channels. This is not a public leaderboard or a new privacy policy.

The gateway checks the session before application frames and at one-second
intervals while waiting for delivery, including replay. Database checks and
writes retain bounded timeouts and fail closed. Pending replay reads keep their
deadline across heartbeat and timer handling. Already authorized/in-flight
frames cannot be retracted; session revocation does not revoke devices, MLS
membership, or copied keys. The heartbeat wire format is unchanged.

Reverting this wave removes the added endpoints and gateway enforcement without
reversing a data migration. Clients using these APIs must handle their absence.
