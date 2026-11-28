# SSH engine: portix-desktop connects through portix_serv

## Intent

`portix-desktop` should connect to servers the same way the Flutter app (`portix_app`) does. Today it runs `/usr/bin/ssh` in a local PTY, and the user types the password every time. The Flutter app instead uses the Rust engine in `portix_serv` (built on `russh`). That engine reads the saved password from Keychain, shows host-key and OTP prompts as dialogs, and supports private keys, ssh-agent, and jump-host chains.

This is the first part of bringing `portix-desktop` up to feature parity with `portix_app`. Later parts (SFTP, port forwarding, Kubernetes, session snapshots) build on the same engine, so one engine serves both apps.

## Goals

- `portix-desktop` opens SSH sessions through `portix_serv`'s `SessionManager` instead of `/usr/bin/ssh`.
- **Password:** read from macOS Keychain, from the entry the Flutter app writes: generic password, service `Portix SSH Profile Password`, account = profile id. If there is no saved password, a dialog asks for it, with a "Save to Keychain" checkbox (on by default). The checkbox writes to the same entry, so the Flutter app sees it too.
- **Private key:** when a profile has `privateKeyPath`, that key is used. If the key is encrypted, a dialog asks for the passphrase. A passphrase saved in Keychain is used first, from the same slot the Flutter app uses (see `ProfileCredentials.useSavedKeyPassphrase`).
- **ssh-agent:** a profile with no password and no key authenticates through `SSH_AUTH_SOCK`.
- **Host key:**
  - Every key is checked against `~/.ssh/known_hosts`.
  - **Unknown host:** a dialog shows the host, algorithm, and fingerprint, with "Cancel" and "Trust and connect". "Trust and connect" writes the key to known_hosts and connects again.
  - **Changed key:** a warning shows the fingerprint and the known_hosts line number. There is no trust button, and the user must fix known_hosts by hand.
  - **Jump hosts:** the dialog names whichever host in the chain was refused.
- **Keyboard-interactive / OTP:** a dialog shows the server's name, instructions, and each prompt. A prompt with `echo = false` gets a hidden field. Submitting the dialog connects again with the answers. A lone hidden "password" prompt is answered with the saved password automatically, as the engine already does.
- **Jump hosts:** chains of any length, as in the Flutter app. A loop or a missing jump profile is refused with an error.
- **Forward agent:** the profile's `forwardAgent` is passed through.
- **Failed connects** (timeout, refused auth, DNS) show a readable error, and the profile grid stays usable.
- **Terminal:** the same `portix-terminal` view, fed by the SSH channel instead of a PTY. Typing, resize, scrollback, paste rules, and reply filtering all behave as they do now.

## Non-goals

- SFTP, port forwarding, session snapshots, Kubernetes, RDP, and Kibana. Each is a later project.
- A profile create/edit form. That is the next project. This one only reads `profiles.json`, plus the optional `privateKeyPath`.
- `startupCommand` and `defaultPath`.
- Generating SSH keys.
- Windows and Linux secret stores. This project is macOS Keychain only. Other platforms ask for the password every time.

## Anti-goals

- **No second SSH engine.** `portix-desktop` must not copy `portix_serv` code. Fixes to the engine land in `portix_serv` and reach both apps.
- **The Flutter app must not change behavior.** Its build keeps working with default features and the same FRB API.
- **No trust-on-first-use without asking.** A changed host key can never be accepted from the UI.
- **No secrets in logs, argv, env, or files other than Keychain and known_hosts.**
- **No network or Keychain I/O on the UI thread.**

## Constraints

- `portix_serv` lives in the public monorepo `github.com/chimeramutate/portix` (default branch `main`; the local checkout sits on `develop` with uncommitted Flutter work). Changes to `portix_serv` go on their own branch in a separate git worktree, so that local work stays untouched. They then get a PR to `develop`.
- `portix-desktop` depends on `portix-serv` by git rev, like the other Portix crates.
- **`portix_serv` changes:**
  - `flutter_rust_bridge`, the `api` module, and `frb_generated` move behind a cargo feature `frb`, which is on by default. `portix-desktop` uses `default-features = false`.
  - The engine (`application`, `domain`, `infrastructure`) must build without FRB.
- **Output must never be dropped.** `SessionManager` today fans terminal output out through a `tokio::sync::broadcast` channel (capacity 1024), and a slow receiver loses bytes (`Lagged`). The desktop path needs lossless, per-session output, for example a per-session `mpsc` receiver returned from `connect`. This change must not break the FRB stream API.
- russh needs a tokio runtime. `portix-desktop` runs one multi-threaded tokio runtime on background threads. gpui talks to it through channels and never blocks on it.
- `portix-terminal` stays SSH-agnostic. It gains a way to drive a `TerminalView` from any byte source and sink (bytes in, bytes out, resize callback, closed signal) next to the existing PTY `Session`. The reply filter, paste rules, OSC cap, and grid clamp apply to both sources.
- Keychain access goes through the `security-framework` crate, not by shelling out to `security` with secrets in argv. macOS will ask once whether `portix-desktop` may read each Flutter-created item ("Always Allow").
- Rust 2024. gpui is reached only through `portix_ui::gpui`.

## Approaches considered

1. **Keep `/usr/bin/ssh` and add autofill through `SSH_ASKPASS`.** Smallest change. But host-key and OTP prompts stay as terminal text, and SFTP, forwarding, and Kubernetes would each need their own `sftp` / `ssh -L` / exec path. Rejected: it works against parity.
2. **Use `portix_serv` as a library, with FRB made optional (chosen).** One engine for both apps. The dialogs match the Flutter app, and later features reuse the engine. Cost: changes in a second repo, and a tokio runtime inside the gpui app.
3. **Copy the engine into a new `portix-ssh` crate.** No changes to the monorepo, but two engines drift. Rejected by the anti-goal.

## Design

### portix_serv (monorepo branch)

- `Cargo.toml`: `[features] default = ["frb"]`, `frb = ["dep:flutter_rust_bridge"]`. Gate `pub mod api;` and `mod frb_generated;` with `#[cfg(feature = "frb")]`.
- `cargo build -p portix-serv` (Flutter path) and `cargo build -p portix-serv --no-default-features` both succeed, and the existing tests pass in both.
- Lossless per-session output for non-FRB callers (builder's call on shape). The prompt state the desktop needs is already public through `infrastructure::host_keys` and `infrastructure::keyboard_interactive`: pending host key, trust, pending prompts, answers. Make the needed functions `pub` if they are `pub(crate)`.
- `SshProfile` gains nothing. The desktop builds it, including the nested `jump_host` chain, from `profiles.json` plus Keychain.

### portix-terminal

- A byte-stream session type (name is the builder's call) that owns a `Term`. It parses incoming bytes on a background thread, the way the PTY event loop does. It sends replies and keystrokes to a provided sink, and it reports resizes to a provided callback.
- `TerminalView::from_session` accepts either kind. The view code and its tests stay as they are.

### portix-desktop

- New module `keychain`: `read_password(profile_id) -> Result<Option<String>>` and `save_password(profile_id, &str) -> Result<()>`, on service `Portix SSH Profile Password`.
- New module `connect`: resolves a profile into a `portix_serv::domain::profile::SshProfile`. It follows the jump chain (loop or missing profile → error), fills in secrets, and runs the connect on the tokio runtime. The result is one of:
  - Connected(session)
  - NeedHostKey { host, port, info }
  - NeedPrompts { host, port, round }
  - NeedPassword { profile_id }
  - NeedPassphrase { profile_id }
  - Failed(message)

  Each `Need…` result opens its dialog. Submitting the dialog retries the connect, and cancelling returns to the grid.
- Dialogs use `portix-ui` components and are modal over the workspace:
  - host-key trust
  - host-key changed (warning only)
  - auth prompts
  - password (with save checkbox)
  - key passphrase

  Hidden fields never show their text, and the strings are dropped after use.
- `profiles.rs`: add an optional `privateKeyPath`.
- `ssh.rs`, which builds the `/usr/bin/ssh` argv, is deleted. Its injection tests no longer apply because no argv is built. Jump-chain validation moves to `connect` with its own tests.
- The Terminals view, the session list, and closing a session are unchanged, except that closing a session calls `disconnect`.

## Security

| # | Risk | Mitigation | Test |
|---|------|-----------|------|
| E1 | MITM on first connect | No silent trust. The fingerprint is shown and the user must confirm. | Unit test: a `NeedHostKey` result never writes known_hosts until trust is called (already covered in `host_keys` tests, kept). |
| E2 | Changed host key accepted | No trust path for a changed key in the UI. | The view test for the changed-key result has no trust action. The engine test that a changed key is refused is kept. |
| E3 | Password leak | Password goes only to Keychain and to russh. It is never logged, never sent through argv or env, and never written to profiles.json. | Grep test: no `println!`/`eprintln!`/`log::` in `connect`/`keychain`. |
| E4 | Wrong Keychain entry | Account = exact profile id. Ids with control characters are refused. | Unit test on the id check. |
| E5 | Jump loop / missing jump | Chain walk with a visited set. | Unit tests: self loop, two-profile loop, missing id, valid 2-level chain. |
| E6 | Output loss → corrupted screen | Lossless per-session output. | portix_serv test: 50 MB through a slow consumer arrives byte-for-byte. |
| E7 | Remote escape sequences | Same reply filter and paste rules as the PTY path. | portix-terminal security tests run against the byte-stream session too. |
| E8 | Agent abuse | Agent is forwarded only when `forwardAgent` is true, and the engine refuses agent channels otherwise. | Existing engine behavior, kept. |
| E9 | Hidden prompt shown | `echo = false` prompts use a masked field. | View test. |

## Settled after the builder check (designer's calls)

1. **Typed connect.** Add a non-FRB entry point to `portix_serv`, for example `SessionManager::connect_awaited`. It awaits the handshake and returns `Result<(SessionInfo, SessionIo), PortixError>`. `SessionIo` carries the lossless output receiver and a close or exit signal. Desktop matches on `PortixError` variants:
   - `HostKeyUnknown`
   - `HostKeyChanged`
   - `KeyboardInteractiveRequired`
   - the passphrase-required and passphrase-incorrect errors
   - `AuthenticationFailed`
   - everything else, shown as a message

   It never parses strings. The FRB `connect` stays as it is.
2. **Global pending state.** The engine keeps its process-global host-key and prompt maps, keyed by host:port. Desktop allows at most one connect attempt in flight per (host, port) and refuses a second one with "already connecting". Cancel calls `forget_pending_host_key` and a new engine function that clears pending prompts and answers for that host:port.
3. **Passphrase.** This follows the Flutter app exactly. Try the key without a passphrase. On "passphrase is required", read the profile's Keychain slot (the same slot as the password). If that is empty or wrong, show the passphrase dialog with a "Save to Keychain" checkbox that writes that slot. Unencrypted keys never touch Keychain.
4. **Password vs agent.** Read `authMethod` from `profiles.json` (`password` | `sshKey`).
   - `password`: read Keychain, and show the dialog if the entry is missing.
   - `sshKey` with `privateKeyPath`: use the key.
   - `sshKey` without a path: use the agent.
   - All 27 saved profiles today are `password`.
5. **Output channel.** A bounded per-session `mpsc` (capacity 256 chunks) is filled from the SSH read loop, next to the existing broadcast. The consumer is portix-terminal's background parser thread, never the UI thread, so the backpressure only slows the remote when parsing falls behind. Session end is signalled by the channel closing, plus a final status in `SessionIo`. The status broadcast is not used.
6. **Cargo.**
   - `crate-type` stays as it is. `main.rs` and the `frb_expand` check-cfg must not need FRB with `--no-default-features`; fix them if they do.
   - The desktop dependency tree has no `curve25519-dalek` today, so no conflict is expected. The builder still confirms with `cargo tree`.
   - Desktop pins the git rev of the monorepo branch head until the PR merges.
7. **Byte-stream terminal.**
   - Resize is fire-and-forget. The UI calls the callback, which uses `try_send` and drops the resize if the queue is full. The next resize carries the latest size.
   - When the session closes, the view shows the same "[process exited]" state as the PTY path and keeps scrollback.
8. **Failure surface.** Use the existing `open_error` banner on the profile grid, which already exists for spawn errors. There is no retry button, because pressing Enter again retries.

## Left to the builder

- The exact shape of the lossless output API in `portix_serv`.
- The name and module layout of the byte-stream session in `portix-terminal`.
- Dialog layout details, within `portix-ui`'s existing components.
- How the tokio runtime is started and shut down. It must stop cleanly on Cmd-Q after every session is disconnected.
