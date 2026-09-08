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
| New command, result variant, or required behavior | Negotiate support before using it, or bump the generation. There is currently no capability negotiation. Unknown commands must never become successful no-ops. |
| Removed/renamed required field, changed wire shape, or incompatible semantics | Bump the generation and document the migration. |

Serde already ignores extra struct fields. Use `Option<T>` or field-level
`#[serde(default)]` for additive fields that can be omitted. Do not default whole
requests or required identifiers: malformed operations must remain errors.

`compatibility_tests.rs` covers missing optional metadata, future fields and error
codes, preserved request IDs, and rejection of missing required data. The client
socket test ensures a future error does not terminate the event stream. Keep these
fixtures when evolving the protocol; a round trip through one version alone does
not establish compatibility. The privileged helper has its own separate protocol.
