# IPC compatibility

`PROTOCOL_VERSION` identifies an incompatible wire generation, not an application
release or feature revision. The current generation is **7**; it removes retired
encrypted-backup commands and responses. Do not accept older generations.

The GUI and daemon run separately. A window can reconnect to a daemon left running
by an earlier build. A changed wire generation fails the handshake so requests
cannot cross incompatible binaries; additive changes within a generation must
remain safe in both directions.

`Hello.channel` and `Hello.maintenance` are required. Daemons reject a different
channel before claiming or activating the primary window. Dev also requires
`dev_instance_v1` in the welcome snapshot; this identifies the build channel
independently of debug or release optimization.

Dev accepts a maintenance connection alongside its primary GUI and limits it
to `QuitApplication`. `make dev-stop` checks the Dev capability before sending
that request, then waits for graceful shutdown, socket removal and release of the
instance lock. Stable daemons reject maintenance connections.

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

General application setting writes preserve proxy preferences, which are edited
through a separate command. The wire protocol no longer accepts encrypted-backup
requests or responses.

## Runtime recovery

The GUI preserves its window and drafts after an unexpected EOF, retries the same socket with a 250 ms to 5 s backoff, and replaces the request sender only after a fresh handshake. It never unlinks the socket or spawns another daemon during recovery. If the daemon crashed, reopen Verge to start it; the retained window reconnects when it becomes available. Normal shutdown sends `Closed { reason: "daemon_shutdown" }`; protocol rejection leaves a visible message instead of silently discarding drafts.

Each socket session owns its reader, writer, subscriptions and command failure state. GUI requests are bounded to 32 queued/pending items and use nonblocking sends. On disconnect, pending UI operations are cleared and old responses are fenced out; fresh reads and subscriptions replace state. Unconfirmed writes are never replayed. Reopening a loading editor is required after a failed load; an already loaded draft remains editable.

Disk logs use a bounded local socket and writer thread. All GUI/daemon writers lock a shared file before opening and appending to the active log, so rotation cannot strand another writer on an old inode. Current log plus three archives are capped at 2 MiB each. Abrupt process exit may lose the final buffered log bytes.

AI uses the `ai_chat_v1` capability on generation 7. Commands cover configuration,
provider testing, starting/cancelling a turn, clearing history and querying state.
Responses identify the operation without echoing the request or its key. Only
connections that request AI receive AI snapshots; snapshots are coalesced at the
100 ms daemon tick, revisions suppress stale results, and final state remains
queryable after reconnection. Disconnect cancels the active turn; the GUI never
automatically repeats inference. Provider IO and config reads/writes run on a worker. AI keys stay in the local
`settings.json` file under `ai` and are never returned in snapshots or settings
exports. Shared settings writes lock and reload the file before updating their
own fields. The AI worker reads only `settings.json.ai`.

`ai_chat_ux_v2` adds `Retry`, which regenerates the last turn without appending
a second user question. New GUIs gate this command. `AiSnapshot.operation` and
per-message evidence are additive, defaulted fields; old snapshots remain readable.
Provider tests retain chat evidence. Drafts are consumed only after an accepted
command response; saving and testing are sequenced after successful persistence.


`ai_actions_v1` adds `Approve { id, digest }` and `Dismiss { id }` on generation 7.
`AiSnapshot.proposals` is additive and defaults to an empty list. The GUI gates
both commands; the dispatcher rejects confirmation from non-UI actors. Only a
locally created proposal can be consumed. Its random ID and digest bind the
operation, parameters, exact candidate, captured state and five-minute expiry;
consumption is single-use. The provider tool schema exposes proposal preparation,
never confirmation. Clear, retry, a new turn, provider changes and failed/cancelled
turns invalidate pending proposals. Reconnection queries their state without
replaying writes. Diagnostic IO, script compilation and validation run on AI
workers; confirmed writes use the daemon's serialized config/runtime transaction
path and report applied, rejected, restored or recovery-failed outcomes. Raw
compiler/controller errors are not sent to the model or proposal UI.
