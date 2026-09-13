# S3-WALLET: wallet history continuation

`GET /v1/wallet/history` requires the existing bearer session. The response
remains `{ "entries": [...] }`, with the same transaction ID, kind, signed
amount and timestamp fields. No balance, transfer, currency or issuance rules
change.

- `limit` defaults to 50. Positive values above 100 are capped at 100; zero,
  negative, non-integer and out-of-range integers receive HTTP 400.
- Optional `before` is a transaction UUID from the caller's own history.
  Omit it for the newest page. Pass the last entry's `transaction_id` to fetch
  the next page, retaining the desired limit.
- Order is `(created_at DESC, transaction_id DESC)`. The boundary is exclusive
  on the whole tuple. The server resolves the anchor's timestamp from the
  caller's own posting, preserving PostgreSQL timestamp precision and ties.
- A short page completes the currently visible traversal. A full page may
  require one more request returning an empty page. There is no next-cursor
  field and no lookahead metadata from another wallet.
- A malformed/empty UUID receives HTTP 400. An unknown UUID and an unrelated
  other user's transaction UUID produce the same HTTP 400 JSON error:
  `{"error":{"code":"bad_request","message":"invalid history cursor"}}`.
  No timestamp, kind, amount or existence detail is returned for foreign
  anchors. A shared transfer is a valid anchor for either participant, and
  each sees only their own signed posting.
- Each request reads live committed state, not a frozen multi-request snapshot.
  Concurrent commits sorting ahead of the boundary are excluded until the
  client refreshes page one; commits sorting behind it can appear on later
  pages. Transaction timestamps and caller-selected UUIDs are not commit order.
  Existing ledger transactions have no application update/delete route.

Example continuation:

```http
GET /v1/wallet/history?limit=100
Authorization: Bearer <session>

GET /v1/wallet/history?limit=100&before=<last-entry-transaction-id>
Authorization: Bearer <session>
```

No schema or dependencies change. Rolling back the server restores the old
limit-only behavior (old servers ignore `before`); clients using continuation
must upgrade with the server or detect repeated entries. This change does not
introduce real-money semantics or public mint/grant routes.
