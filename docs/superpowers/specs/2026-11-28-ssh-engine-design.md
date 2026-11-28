# SSH engine: new portix-ssh and portix-rdp crates

## Intent

`portix-desktop` should connect to servers the same way the Flutter app (`portix_app`) does. Today it runs `/usr/bin/ssh` in a local PTY, and the user types the password every time. The Flutter app instead uses the Rust engine in `portix_serv` (built on `russh`). That engine reads the saved password from Keychain, shows host-key and OTP prompts as dialogs, and supports private keys, ssh-agent, and jump-host chains.

This is the first part of bringing `portix-desktop` up to feature parity with `portix_app`. The engines are copied out of the monorepo into two new standalone crates, `portix-ssh` (from `portix_serv`) and `portix-rdp` (from `portix_rdp`), with Flutter Rust Bridge removed. The old project (`portix_app`, `portix_serv`, `portix_rdp`) is not touched. Later parts (SFTP, port forwarding, Kubernetes, snapshots, RDP) build on these crates.

## Goals

- New repo `portix-ssh` holds the SSH engine and is FRB-free. `portix-desktop` opens SSH sessions through its `SessionManager` instead of `/usr/bin/ssh`.
- New repo `portix-rdp` holds the RDP engine and is FRB-free. It builds and passes its tests on its own, but this project does not wire it into the desktop app (see Constraints).
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

- SFTP, port forwarding, session snapshots, Kubernetes, Kibana, and RDP in the desktop UI. Each is a later project. The SFTP and port-forward code comes along in `portix-ssh` unchanged, but no desktop UI uses it yet.
- A profile create/edit form. That is the next project. This one only reads `profiles.json`, plus the optional `privateKeyPath`.
- `startupCommand` and `defaultPath`.
- Generating SSH keys.
- Windows and Linux secret stores. This project is macOS Keychain only. Other platforms ask for the password every time.

## Anti-goals

- **The old project is not touched.** No commits, branches, or files change in `~/Documents/project/portix`. The code is copied out, not moved.
- **No trust-on-first-use without asking.** A changed host key can never be accepted from the UI.
- **No secrets in logs, argv, env, or files other than Keychain and known_hosts.**
- **No network or Keychain I/O on the UI thread.**

## Constraints

- **Source of the copy.** Monorepo `github.com/chimeramutate/portix` at commit `914a92d` (portix_serv and portix_rdp last changed in `6099fce`).
  - The first commit of each new repo is that source unmodified.
  - `FORK.md` records where the code came from and every patch made after it, as in `portix-terminal`.
  - The new repos are accepted to drift from the monorepo. Engine fixes made in one place are not copied to the other automatically.
- **New repos:**
  - `~/Documents/project/portix-ssh`: package `portix-ssh`, lib `portix_ssh`.
  - `~/Documents/project/portix-rdp`: package `portix-rdp`, lib `portix_rdp`.
  - Both on GitHub under `chimeramutate`, private (same as `portix-terminal`), and pushed only after approval.
  - `portix-desktop` depends on `portix-ssh` by git rev.
- **FRB removed from both:**
  - `flutter_rust_bridge` is dropped as a dependency.
  - The `api` modules and `frb_generated.rs` are deleted, along with the FRB-only `StreamSink` stream helpers.
  - `crate-type` becomes plain `lib`.
  - The engine modules (`application`, `domain`, `infrastructure`) keep their code and tests.
  - Any helper from `api` that the desktop needs (for example `HostKeyInfo` or the pending-prompt lookups) moves into the engine as a plain Rust function.
- **SSH and RDP cannot share one binary.** `russh` pins `curve25519-dalek =5.0.0-rc.0` and IronRDP pins `=5.0.0-rc.1`, and Cargo cannot hold both in one dependency tree. This is why the monorepo builds them as two separate libraries. `portix-desktop` therefore depends only on `portix-ssh` in this project. Bringing RDP into the desktop later will need a separate helper process built from `portix-rdp` (to be designed in the RDP project).
- **Output must never be dropped.** `SessionManager` today fans terminal output out through a `tokio::sync::broadcast` channel (capacity 1024), and a slow receiver loses bytes (`Lagged`). The broadcast streams existed for FRB. `portix-ssh` replaces them with lossless per-session output (see Settled 5).
- russh needs a tokio runtime. `portix-desktop` runs one multi-threaded tokio runtime on background threads. gpui talks to it through channels and never blocks on it.
- `portix-terminal` stays SSH-agnostic. It gains a way to drive a `TerminalView` from any byte source and sink (bytes in, bytes out, resize callback, closed signal) next to the existing PTY `Session`. The reply filter, paste rules, OSC cap, and grid clamp apply to both sources.
- Keychain access goes through the `security-framework` crate, not by shelling out to `security` with secrets in argv. macOS will ask once whether `portix-desktop` may read each Flutter-created item ("Always Allow").
- Rust 2024. gpui is reached only through `portix_ui::gpui`.

## Approaches considered

1. **Keep `/usr/bin/ssh` and add autofill through `SSH_ASKPASS`.** Smallest change. But host-key and OTP prompts stay as terminal text, and SFTP, forwarding, and Kubernetes would each need their own `sftp` / `ssh -L` / exec path. Rejected: it works against parity.
2. **Use `portix_serv` as a library with FRB made optional.** One engine for both apps, but it changes the old project. Rejected by the user: the old project must stay untouched.
3. **Copy the engines into new `portix-ssh` and `portix-rdp` crates (chosen by the user).** The old project stays as it is, and the new crates can change freely for the desktop. Cost: the two engines drift over time. A tokio runtime runs inside the gpui app.

## Design

### portix-ssh (new repo)

- Copied from `portix_serv`, with FRB removed (see Constraints).
- `cargo build` and `cargo test` pass. The e2e tests that need a live server keep whatever gating they have today.
- Typed, awaited connect and lossless per-session output (Settled 1 and 5). The broadcast streams are removed.
- The pending host-key and prompt functions are `pub`, plus a function that clears pending prompts and answers (Settled 2).
- `SshProfile` gains nothing. The desktop builds it, including the nested `jump_host` chain, from `profiles.json` plus Keychain.

### portix-rdp (new repo)

- Copied from `portix_rdp`, with FRB removed.
- `cargo build` and `cargo test` pass.
- No other changes in this project.

### portix-terminal

- A byte-stream session type (name is the builder's call) that owns a `Term`. It parses incoming bytes on a background thread, the way the PTY event loop does. It sends replies and keystrokes to a provided sink, and it reports resizes to a provided callback.
- `TerminalView::from_session` accepts either kind. The view code and its tests stay as they are.

### portix-desktop

- New module `keychain`: `read_password(profile_id) -> Result<Option<String>>` and `save_password(profile_id, &str) -> Result<()>`, on service `Portix SSH Profile Password`.
- New module `connect`: resolves a profile into a `portix_ssh::domain::profile::SshProfile`. It follows the jump chain (loop or missing profile → error), fills in secrets, and runs the connect on the tokio runtime. The result is one of:
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
| E6 | Output loss → corrupted screen | Lossless per-session output. | portix-ssh test: 50 MB through a slow consumer arrives byte-for-byte. |
| E7 | Remote escape sequences | Same reply filter and paste rules as the PTY path. | portix-terminal security tests run against the byte-stream session too. |
| E8 | Agent abuse | Agent is forwarded only when `forwardAgent` is true, and the engine refuses agent channels otherwise. | Existing engine behavior, kept. |
| E9 | Hidden prompt shown | `echo = false` prompts use a masked field. | View test. |

## Settled after the builder check (designer's calls)

1. **Typed connect.** `portix-ssh`'s `SessionManager::connect` It awaits the handshake and returns `Result<(SessionInfo, SessionIo), PortixError>`. `SessionIo` carries the lossless output receiver and a close or exit signal. Desktop matches on `PortixError` variants:
   - `HostKeyUnknown`
   - `HostKeyChanged`
   - `KeyboardInteractiveRequired`
   - the passphrase-required and passphrase-incorrect errors
   - `AuthenticationFailed`
   - everything else, shown as a message

   It never parses strings.
2. **Global pending state.** The engine keeps its process-global host-key and prompt maps, keyed by host:port. Desktop allows at most one connect attempt in flight per (host, port) and refuses a second one with "already connecting". Cancel calls `forget_pending_host_key` and a new engine function that clears pending prompts and answers for that host:port.
3. **Passphrase.** This follows the Flutter app exactly. Try the key without a passphrase. On "passphrase is required", read the profile's Keychain slot (the same slot as the password). If that is empty or wrong, show the passphrase dialog with a "Save to Keychain" checkbox that writes that slot. Unencrypted keys never touch Keychain.
4. **Password vs agent.** Read `authMethod` from `profiles.json` (`password` | `sshKey`).
   - `password`: read Keychain, and show the dialog if the entry is missing.
   - `sshKey` with `privateKeyPath`: use the key.
   - `sshKey` without a path: use the agent.
   - All 27 saved profiles today are `password`.
5. **Output channel.** A bounded per-session `mpsc` (capacity 256 chunks) is filled from the SSH read loop and replaces the broadcast. The consumer is portix-terminal's background parser thread, never the UI thread, so the backpressure only slows the remote when parsing falls behind. Session end is signalled by the channel closing, plus a final status in `SessionIo`. The status broadcast is not used.
6. **Cargo.**
   - Drop `src/main.rs` and the `frb_expand` check-cfg lint.
   - The desktop dependency tree has no `curve25519-dalek` today, so no conflict is expected for `portix-ssh` alone. The builder still confirms with `cargo tree`.
   - Desktop pins the git rev of `portix-ssh` `main`. Until it is pushed, a `path` dependency is used, as was done for `portix-terminal`.
7. **Byte-stream terminal.**
   - Resize is fire-and-forget. The UI calls the callback, which uses `try_send` and drops the resize if the queue is full. The next resize carries the latest size.
   - When the session closes, the view shows the same "[process exited]" state as the PTY path and keeps scrollback.
8. **Failure surface.** Use the existing `open_error` banner on the profile grid, which already exists for spawn errors. There is no retry button, because pressing Enter again retries.

## Left to the builder

- The exact shape of the lossless output API in `portix-ssh`.
- The name and module layout of the byte-stream session in `portix-terminal`.
- Dialog layout details, within `portix-ui`'s existing components.
- How the tokio runtime is started and shut down. It must stop cleanly on Cmd-Q after every session is disconnected.
