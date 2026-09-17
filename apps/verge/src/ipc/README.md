# IPC compatibility

`PROTOCOL_VERSION` identifies an incompatible wire generation, not an application
release or feature revision. Keep generation **6** for compatible changes. Versions
before this baseline do not support unknown error codes, so do not lower the
constant or accept arbitrary older versions.

The GUI and daemon run separately. A window can reconnect to a daemon left running
by an earlier build. Compatibility therefore needs to work in both directions.

| Change | Handling |
| --- | --- |
| UI, timeout, implementation, or bug fix | Keep the version. |
| New error code | Keep the version. `ErrorCode::Unknown` preserves the enclosing error and its message. Do not infer recovery actions or core health from it. |
| New response metadata or optional field | Keep the version if older readers can ignore it and newer readers have a safe default when it is absent. Add a compatibility fixture. |
| New command option | Keep the version only if omission preserves old behavior and an old daemon ignoring it is safe. Security, confirmation, or correctness requirements must not depend on an ignored field. |
| New command, result variant, or required behavior | Negotiate support before using it, or bump the generation. `InitialSnapshot.capabilities` advertises supported additions; missing means none. Unknown commands must never become successful no-ops. |
| Removed/renamed required field, changed wire shape, or incompatible semantics | Bump the generation and document the migration. |

Serde already ignores extra struct fields. Use `Option<T>` or field-level
`#[serde(default)]` for additive fields that can be omitted. Do not default whole
requests or required identifiers: malformed operations must remain errors.

`compatibility_tests.rs` covers missing optional metadata, future fields and error
codes, preserved request IDs, and rejection of missing required data. The client
socket test ensures a future error does not terminate the event stream. Keep these
fixtures when evolving the protocol; a round trip through one version alone does
not establish compatibility. The privileged helper has its own separate protocol.

The `unified_system_proxy` capability gates the new `SetEnabled` and `UpdateSystemProxySettings` commands. Older daemons omit it; the GUI asks the user to restart
before dispatching either write. Legacy per-protocol commands keep their meanings. General application setting
writes preserve proxy preferences, so an older GUI cannot reset them by omitting
the new field.

Encrypted backup export and restore have been retired. Generation-6 request
variants remain decodable and return `NotFound` without accessing backup data.
The new GUI has no backup actions; the old export result remains decode-only so
mixed builds do not disconnect solely because of this retired message.

## Runtime recovery

The GUI preserves its window and drafts after an unexpected EOF, retries the same socket with a 250 ms to 5 s backoff, and replaces the request sender only after a fresh handshake. It never unlinks the socket or spawns another daemon during recovery. If the daemon crashed, reopen Verge to start it; the retained window reconnects when it becomes available. Normal shutdown sends `Closed { reason: "daemon_shutdown" }`; protocol rejection leaves a visible message instead of silently discarding drafts.

Each socket session owns its reader, writer, subscriptions and command failure state. GUI requests are bounded to 32 queued/pending items and use nonblocking sends. On disconnect, pending UI operations are cleared and old responses are fenced out; fresh reads and subscriptions replace state. Unconfirmed writes are never replayed. Reopening a loading editor is required after a failed load; an already loaded draft remains editable.

Disk logs use a bounded local socket and writer thread. All GUI/daemon writers lock a shared file before opening and appending to the active log, so rotation cannot strand another writer on an old inode. Current log plus three archives are capped at 2 MiB each. Abrupt process exit may lose the final buffered log bytes.

AI uses the `ai_chat_v1` capability on generation 6. Commands cover configuration,
provider testing, starting/cancelling a turn, clearing history and querying state.
Responses identify the operation without echoing the request or its key. Only
connections that request AI receive AI snapshots; snapshots are coalesced at the
100 ms daemon tick, revisions suppress stale results, and final state remains
queryable after reconnection. Disconnect cancels the active turn; the GUI never
automatically repeats inference. Provider IO and Keychain work run on a worker.
