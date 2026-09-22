# Profile scripts

Open **Profiles → Global script** for a transform applied to every profile, or
**Profile script** on a profile card for a transform applied only to that profile.
The editor opens with a minimal template. **Save draft** persists edits without
changing the enabled version. **Preview result** shows the generated YAML and
console messages for the selected preview profile. **Save and enable** validates
the affected configurations and reloads the current profile; **Disable** removes
that transform from the pipeline while retaining its draft.

```javascript
function main(config, profileName, context) {
  config.rules = ["DOMAIN,example.com,DIRECT", ...(config.rules ?? [])];
  console.log(profileName, context.profileId);
  return config;
}
```

The entry point receives the configuration object, profile name, and
`{ profileId, apiVersion: 1 }`. One- and two-argument functions also work. Return a
plain object containing JSON-compatible values. Promises, asynchronous work,
accessors, cycles, undefined, non-finite numbers, unsafe integers, tagged YAML,
and non-string YAML keys are rejected rather than silently converted.

The order is source YAML → global Merge → global script → profile script →
Network overrides → private runtime controller/TUN settings. Script output does
not overwrite the imported subscription YAML. A content-addressed private cache
reuses the same result for matching inputs, validation, activation, refreshes,
and rollback. Changing a script, source YAML, Merge, or profile name changes those
inputs. Network overrides are applied afterward. A preview is a transform preview;
activation also runs Mihomo validation and checks core health.

Boa 0.22 runs in a short-lived subprocess. There are no filesystem, network,
process, timer, or module-loading host APIs. The worker never drains Promise jobs.
The parent kills and reaps workers after five seconds, and on macOS monitors RSS
against 256 MiB; RSS sampling is a soft limit, not a hard heap sandbox. Scripts are
limited to 256 KiB each, worker input to 12 MiB, JSON result to 4 MiB, preview wire
output to 8 MiB, and console output to 256 records / 64 KiB. Boa also limits loop
iterations, recursion, VM stack, and buffer allocation. The clock is fixed at the
Unix epoch; scripts should operate on configuration data instead of wall time.

Logs and detailed preview errors remain in the editor/private compilation cache.
Automatic application failures report a generic script error. Private caches and
script versions are stored below the profile store's `scripts/` and `compiled/`
directories, using mode 0600 files. Old candidates are bounded and the previous
compiled version is protected during updates.

Execution is synchronous in the daemon's existing configuration command path.
An expensive script can delay daemon commands while it runs (up to five seconds
per affected profile). The GUI remains a separate process. This is a configuration
transform facility, not a general-purpose plugin or browser runtime.

Existing profiles can be edited using **Edit details** (name, subscription URL,
interval, User-Agent) and **Edit YAML**. A blank update interval selects manual
updates. A changed subscription URL is used on the next download. YAML saves and
manual subscription updates also work before Mihomo starts, using its validation
mode before persisting the source.
