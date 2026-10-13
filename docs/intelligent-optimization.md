# Intelligent optimization

Overview exposes **Check network** and **Optimize** outside the scrolling area.
The network-check sheet separates results, history and observation preferences.
Checks and node recovery do not require a model, API key, or kernel modification.

## Results and next steps

The fixed result header names the observed problem and offers the next relevant
action: activate a profile, retry the core, inspect network settings, read error
logs or compare alternatives. It never substitutes a general disclaimer for the
result. Checks show progress as evidence arrives, with cancellation kept visible.

Local profile/core/system-proxy checks use compact status rows. Node results group
the latest Google and Cloudflare probe by node; previous rounds, configuration-only
DNS/TUN evidence and raw target URLs are behind **Show check details**. Untested
nodes are not reported healthy. History is read-only and explicitly historical.

When probes have not been permitted, **Test nodes** opens a contextual permission
prompt. **Allow and test now** persists permission and starts the check in one
daemon command; dismissing the prompt never grants permission. Background settings
and optional AI consent remain separate from the local result.
Opening comparison brings the group picker and next action to the top, ahead of
long result lists. Group choice is explicit and resets for a new report or a newly
opened comparison; a failed node alone does not identify a unique editable group.
Pending proposals lead to review rather than another check.
No replacement found and insufficient evidence for switching have separate conclusions,
so an unsuccessful comparison does not simply send the user back into it.
An inconclusive comparison means the two-target, two-round failure condition was
not met; it does not imply temporal fluctuation. Operation errors are visible on
every tab. Clearing is available even when only the current report exists.
Terminal check failures are stored as a bounded finding in the report so history
cannot turn them into success. Raw controller error text is not persisted.

## Checks

- Check selected profile, core controller, managed system-proxy state, TUN
  configuration/helper connection and DNS configuration.
- Resolve selected group chains with cycle/depth guards. With active-probe consent,
  probe up to three unique leaf nodes sequentially against fixed Google and Cloudflare HTTPS 204 targets.
  Direct mode skips proxy probes. Unsupported/missing evidence is not a pass.
- Observe error and warning-level TCP/UDP dial-failure timestamps for five minutes and recent connection counts when
  background observation is enabled. Do not store raw log bodies, destinations or
  process histories in optimization reports.
- Return incremental findings; cancellation preserves completed evidence. The
  30-second check / 45-second comparison budgets stop admitting probes near the
  deadline, not hard wall-clock deadlines for all OS/controller reads.

Node probes do **not** prove browser connectivity, OS DNS correctness, TUN routing,
DNS leak prevention or access to a user's website. Probe failures are observations,
not proof of a particular cause. Probes consume subscription traffic; request-count
limits are not byte limits. Mihomo handles the fixed HTTPS requests.

Active probes have a separate, persistent opt-in, off by default. Without it, checks
read local state and report node connectivity as untested; comparison and application
are rejected by the daemon. Existing background permissions do not grant this opt-in.
Mihomo's delay API updates shared health and latency records: Fallback, URLTest and
other automatic groups may react by changing selection and the exit IP for new
connections, including when a probed node is shared with a manual group. This is core
behavior, not an isolated read-only test. The permission and its impact are shown
before enabling it. Disabling it prevents future probes, not reversal of old effects.
Capability `optimization_v3` gates the combined permission-and-check command and
keeps new clients from using an older daemon with different interaction semantics.

## Confirmed optimization

1. Choose one Selector with a directly selected leaf member. Nested/automatic
   groups and built-in DIRECT/REJECT/PASS nodes are not switching candidates.
2. Keep a working node. Only failure on **both targets in two rounds** starts a
   comparison of up to three direct leaf alternatives; a replacement must pass
   both targets. This release recovers unavailable nodes, not merely slow ones.
3. Show the old/new selection and exit-IP impact. Manual Selector confirmation is
   separate from active-probe consent. Confirmation binds to an opaque,
   single-use proposal valid for two minutes, the profile inputs, group topology,
   mode, network settings, core socket and observed interface-address fingerprint.
4. Persist a minimal intent before writing, read back selection, then probe again.
   On failed probes, restore the old selection only if the environment still
   matches. Never overwrite an observed external change. Uncertain outcomes remain
   uncertain and require inspection; an interrupted intent is never replayed.

During verification the daemon defers conflicting writes and scheduled updates.
Shutdown waits for the bounded controller transaction before stopping the core.
No DNS, TUN, PAC, mode or routing-rule changes are made. Existing connections remain
open. Controller APIs have no compare-and-swap, so external controllers can still
race the final read/write boundary. Interface addresses do not uniquely identify
Wi-Fi networks; same-address gateway/AP changes are not comprehensively detected.

## Background and AI

Background observation, active probing and automatic model analysis are separate
opt-ins, all off by default. The daemon checks every 10–20 minutes and, when active
probes are allowed, rotates through selected nodes, one per run. The background
cursor survives restarts and daily budget resets; manual checks use a separate
rotating cursor. Individual reports still mark incomplete coverage, and topology
changes can change which node comes next. The daemon waits 60 seconds
after observed interface changes/wake gaps, and can schedule a check after an error
burst. Budgets of 144 background checks and 10 automatic analyses per UTC day survive
restart. This release does not issue unattended Selector writes. Automatic groups
can still react to explicitly permitted probes as described above.

Manual AI analysis requires confirmation of the data scope. Automatic analysis uses
the separately enabled permission only on abnormal reports. Both reuse the provider
configured on Intelligence, with a 30-second maximum model timeout and no tools.
Only check codes, states, anonymous per-report node indices, probe timings, outcome
codes and report timestamps are sent. No node/group names, raw logs, browsing domains
or configuration are uploaded. Provider fees may apply. Model failure does not erase
local results, and model output cannot authorize changes.

`optimization.json` stores settings, budgets and up to 24 anonymized reports for
seven days in the existing app data directory. An unfinished
`optimization-intent.json` raises an uncertainty warning on restart. **Clear local
reports** clears report history and acknowledges that warning, not daily budgets.

## Verification and follow-up

The test suite covers redaction, bounded probes, cancellation, eligibility,
confirmation expiry, stale state, write/readback failures, rollback, external
changes, persisted budgets, user-only IPC and actual GPUI button/sheet interaction.
An opt-in pinned-Mihomo test exercises a real switch and failed-probe rollback
using closed loopback proxy endpoints without altering system networking:

```sh
MIHOMO_BIN=/path/to/pinned/mihomo MACOSX_DEPLOYMENT_TARGET=15.0 \
  cargo +1.97.1 test -p verge optimization_failed_probe_restores_selection_on_pinned_mihomo --locked -- --ignored
```

Still future work: per-group unattended delegation, latency hysteresis and cooldown
policies, comprehensive OS-network/DNS diagnosis, richer historical trends, real
long-running power/traffic measurements and Jev shadow comparisons. Jev is not a
dependency of the local health path.
