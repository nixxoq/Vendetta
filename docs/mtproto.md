# Telegram MTProto Protocol Specification & Invariants

This document specifies the Telegram MTProto protocol rules, MTProto API methods, serialization types, error handling contracts, and ingestion invariants used by Vendetta.

---

## 1. Identity and Addressing Model

Canonical message identity is defined as the tuple:

$$\text{CanonicalMessageId} = (\text{peer\_id}, \text{message\_id})$$

* Bare `message_id` values are never assumed to be globally unique; message IDs are scoped to their containing peer.
* Peer entities are partitioned into three distinct types:
  * `PeerUser(user_id)`: Direct messaging (one-on-one) dialogs.
  * `PeerChat(chat_id)`: Legacy basic groups.
  * `PeerChannel(channel_id)`: Supergroups, broadcast channels, and forum groups.
* Peer access hashes must be preserved across sessions to permit subsequent API queries.

---

## 2. Authentication & Session Lifecycle

The primary MTProto client is `grammers-client` (pinned version `0.10.0`), encapsulated behind `vendetta_tg_adapter`.

### Authentication Flow
1. **Interactive / CLI Flow**:
   * Request phone number in international format (`+<country><number>`).
   * Invoke `auth.sendCode` to generate a login token and SMS/app verification code.
   * Invoke `auth.signIn(phone_number, phone_code_hash, phone_code)`.
   * If a Two-Factor Authentication (2FA) cloud password is required (`SESSION_PASSWORD_NEEDED`), invoke `auth.checkPassword` with the computed SRP hash.
2. **Session Persistence**:
   * Export MTProto authorization key and server DC addresses to `session.json`.
   * Files are stored per account in the workspace layout.
3. **Session Revocation**:
   * `vendetta auth logout`: Executes `auth.logOut` over MTProto to terminate the session on Telegram servers, then removes local session files.
   * `vendetta auth logout --local-only` (or `vendetta auth forget`): Removes local session files without contacting Telegram servers.

---

## 3. History Retrieval & Pagination

### Core Methods

| Method | Parameters | Primary Purpose |
| :--- | :--- | :--- |
| `messages.getHistory` | `peer`, `offset_id`, `offset_date`, `add_offset`, `limit`, `max_id`, `min_id`, `hash` | Sequential page retrieval of message history for a peer. |
| `messages.getMessages` | `id: Vec<InputMessage>` | Exact retrieval of specific messages by ID across peers. |
| `messages.search` | `peer`, `q`, `filter`, `min_date`, `max_date`, `offset_id`, `add_offset`, `limit`, `max_id`, `min_id`, `hash` | Server-side filtered query execution. |

### Invariants
* **Transport Independence**: Pagination order is strictly a retrieval mechanism and must never be used to infer message relationships, reply threads, or albums.
* **Idempotent Ingestion**: Duplicate messages returned across overlapping pagination windows are deduplicated on `(peer_id, message_id)`.
* **Exact Target Resolution**: Missing reply targets and thread roots are resolved via targeted `messages.getMessages` calls rather than full-history re-scans.

---

## 4. Reply & Thread Graph Resolution

Reply metadata is deserialized from `tl::types::MessageReplyHeader`:

| Field | Type | Description |
| :--- | :--- | :--- |
| `reply_to_msg_id` | `Option<i32>` | Target message ID within the current or target peer. |
| `reply_to_top_id` | `Option<i32>` | Root message ID of a discussion thread or forum topic. |
| `reply_to_peer_id` | `Option<Peer>` | Target peer pointer for cross-chat or channel discussion replies. |
| `forum_topic` | `bool` | Flag indicating whether the message belongs to a forum topic. |
| `quote_text` | `Option<String>` | Quoted text slice from the reply target. |

### Taxonomy of Relationships
1. **Direct Reply**: `reply_to_msg_id.is_some()`, `reply_to_top_id.is_none()`, `forum_topic == false`.
2. **Comment Thread**: `reply_to_top_id.is_some()`, `forum_topic == false`.
3. **Forum Topic**: `forum_topic == true` or `reply_to_top_id` pointing to a `MessageActionTopicCreate` service message.
4. **Channel Discussion**: `reply_to_peer_id.is_some()`, routing replies to a linked supergroup.

---

## 5. Media Engine & Transfer Protocol

### Binary Download Coverage (`upload.getFile`)

| Media Variant | Storage Class | Download Mechanism | Support Status |
| :--- | :--- | :--- | :--- |
| `MessageMediaPhoto` | Downloadable Binary | `InputPhotoFileLocation` | Full (highest resolution `Size` or `Progressive`) |
| `MessageMediaDocument` | Downloadable Binary | `InputDocumentFileLocation` | Full |
| `MessageMediaWebPage` | Metadata + Media | `InputFileLocation` if photo/doc attached | Full (attached binary extracted) |
| `MessageMediaGame` | Metadata + Media | Cover photo/animation location | Full (attached binary extracted) |
| `MessageMediaPaidMedia` | Metadata + Media | Nested photo/document location | Full (unlocked items extracted) |
| `MessageMediaStory` | Metadata + Media | Story photo/document location | Full (story binary extracted) |
| `MessageMediaContact` | Metadata Only | N/A | Preserved in SQLite `raw_tl` |
| `MessageMediaGeo` | Metadata Only | N/A | Preserved in SQLite `raw_tl` |
| `MessageMediaVenue` | Metadata Only | N/A | Preserved in SQLite `raw_tl` |
| `MessageMediaPoll` | Metadata Only | N/A | Preserved in SQLite `raw_tl` |
| `MessageMediaDice` | Metadata Only | N/A | Preserved in SQLite `raw_tl` |
| `MessageMediaInvoice` | Metadata Only | N/A | Preserved in SQLite `raw_tl` |
| `MessageMediaGiveaway` | Metadata Only | N/A | Preserved in SQLite `raw_tl` |
| `MessageMediaUnsupported` | Metadata Only | N/A | Preserved in SQLite `raw_tl` |

### 1 MB Fragment Boundary Invariant
Telegram MTProto file streaming requires that no chunk request cross a 1 MB (1,048,576 bytes) fragment boundary:

$$\left\lfloor \frac{\text{offset}}{1024 \times 1024} \right\rfloor = \left\lfloor \frac{\text{offset} + \text{limit} - 1}{1024 \times 1024} \right\rfloor$$

### Parameter Constraints
* `offset`: Signed 64-bit integer (`i64`).
* `precise: true`:
  * `offset % 1024 == 0` (strictly 1 KB aligned).
  * `limit % 1024 == 0` (strictly 1 KB aligned).
  * `limit <= 1048576` (maximum 1 MB per request).
* `precise: false`:
  * `offset % 4096 == 0` (strictly 4 KB aligned).
  * `limit % 4096 == 0` (strictly 4 KB aligned).
  * `1048576 % limit == 0` (must evenly divide 1 MB).

### Chunk Size Planning Formula
Given a verified `offset` ($k \times 1024$):

$$\text{fragment\_offset} = \text{offset} \pmod{1048576}$$

$$\text{remaining} = 1048576 - \text{fragment\_offset}$$

$$\text{limit} = \max\left(1024, \left\lfloor \frac{\min(\text{configured\_size}, \text{remaining})}{1024} \right\rfloor \times 1024\right)$$

### Range Hash Verification (`upload.getFileHashes`)
* Range hashes are queried via `upload.getFileHashes(location, offset)`.
* Received ranges $[O_s, O_s + L_s)$ are verified for contiguous coverage of chunk $[O_c, O_c + L_c)$.
* Each slice `chunk[O_s - O_c .. O_s - O_c + L_s]` is verified against `FileHash.hash` (SHA-256).
* If range hashes are unavailable (`LOCATION_INVALID`), whole-file SHA-256 validation is enforced on final assembly.

### File Reference Expiration
Telegram file locations include an ephemeral `file_reference` token. When `FILE_REFERENCE_EXPIRED` (400) or `FILE_REFERENCE_INVALID` (400) is returned:
1. Identify the originating parent object (Message, Channel, User profile).
2. Query the parent object via `messages.getMessages` or `channels.getChannels` to retrieve fresh metadata.
3. Extract the renewed `file_reference` and update the `InputFileLocation`.
4. Resume chunk transfer from the current durable offset.

---

## 6. Incremental Synchronization & State Sequences

Telegram maintains state across multiple disjoint update sequence numbers:

```text
Global Updates Stream
         |
        seq
         |
   +-----+-------------------------+
   |                               |
Common Message Box           Channel Message Boxes
   |                               |
pts / qts                     per-channel pts
```

* `seq`: Global update sequence counter.
* `pts` / `pts_count`: Primary message box event counter (messages, edits, deletions).
* `qts`: Secondary event counter (secret chats, auxiliary events).
* Channel PTS: Independent event counters allocated per supergroup/channel.

### Reconciliation Methods
* `updates.getDifference(pts, date, qts)`: Reconciles missing common box updates.
* `updates.getChannelDifference(channel, filter, pts, limit)`: Reconciles missing updates for a specific channel.
* `updates.channelDifferenceTooLong`: Triggered when the local `pts` is outside the server ring buffer. Requires a bounded full-history rescan to establish a new baseline state lock.

---

## 7. RPC Error Taxonomy & Fault Handling

| Error Code | Pattern | Action |
| :--- | :--- | :--- |
| `303` | `FILE_MIGRATE_X`, `NETWORK_MIGRATE_X`, `PHONE_MIGRATE_X`, `USER_MIGRATE_X` | Re-issue the request to Data Center $X$. |
| `420` | `FLOOD_WAIT_X`, `FLOOD_PREMIUM_WAIT_X` | Suspend worker execution for $X$ seconds; apply exponential backoff. |
| `400` | `FILE_REFERENCE_EXPIRED`, `FILE_REFERENCE_INVALID` | Refresh file reference from source object; do not retry without refresh. |
| `400` | `OFFSET_INVALID`, `LIMIT_INVALID` | Fatal programming fault; abort chunk request. |
| `401` | `AUTH_KEY_UNREGISTERED`, `SESSION_REVOKED` | Terminate synchronization; prompt for user re-authentication. |

---

## 8. Takeout Session Protocol

Telegram Takeout provides rate-limit relaxed exports for authenticated user accounts:

* **Initialization**: `account.initTakeoutSession` with flags selecting export scopes (`contacts`, `message_users`, `message_chats`, `message_megagroups`, `message_channels`, `files`, `file_max_size`).
* **Invocation**: Wrap MTProto calls inside `invokeWithTakeout(takeout_id, query)`.
* **Termination**: `account.finishTakeoutSession` upon completion or cancellation.
* **Storage Invariant**: Output from Takeout sessions is normalized into the canonical SQLite schema identically to standard history queries.
