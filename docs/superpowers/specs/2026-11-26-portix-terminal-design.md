# portix-terminal: embedded SSH terminal

## Intent

Portix lists SSH profiles but cannot open one. The "Terminals" rail item is disabled. This adds a terminal that runs inside the Portix window and opens an SSH session from a profile.

The terminal lives in its own crate, `portix-terminal`, which is a fork of `alacritty_terminal`. Keeping it as a fork (like `portix-gpui`) means Portix owns the code and can change how it behaves, not only how it looks.

## Goals

- New repo `portix-terminal`: fork of `alacritty_terminal` 0.26.0 from crates.io, plus a gpui view module that draws the terminal with `portix-ui`.
- In `portix-desktop`, pressing Enter or double-clicking a profile card opens a terminal tab running `ssh` for that profile.
- The "Terminals" rail item turns on once a session exists. Its side panel lists the open sessions, and choosing one shows it.
- Typing, colors, cursor, resizing with the window, scrollback (mouse wheel), paste (cmd-v), and mouse selection with copy (cmd-c) all work.

## Non-goals (v1)

- Filling the password automatically from Keychain. The user types the password at the `ssh` prompt.
- RDP, SFTP, Kubernetes, Kibana.
- Split panes, search in scrollback, ligatures, a terminal settings screen.
- Running a profile's `startupCommand` or opening at `defaultPath`.
- Windows support.
- Opening hyperlinks (OSC 8) and writing to the clipboard from the server (OSC 52). Both stay off for security (see S4, S10).

## Anti-goals

- No code copied from Zed's `terminal` / `terminal_view` crates. Those crates are GPL-3. Everything here stays Apache-2.0, the same as gpui and alacritty.
- No PTY reads or writes on the UI thread. A slow or stuck server must never freeze the window.
- Opening a terminal must not lose the profile grid. The user can always switch back to it.

## Constraints

- Stack: Rust 2024, `portix-gpui` reached as `portix_ui::gpui`. Apps never depend on gpui directly.
- `portix-terminal` depends on `portix-ui` through git with a pinned rev. `portix-desktop` must pin **the same** `portix-ui` rev. Otherwise Cargo builds two copies and the `Palette` and gpui types won't match.
- Profiles come from `~/.portix/profiles.json`, which the Flutter app also reads and writes. Portix only reads it. Fields used: `host`, `port`, `username`, `jumpProfileId`, `forwardAgent`, `terminalFontSize`. Every saved profile currently has `authMethod: password`.

## Approaches considered

1. **Fork `alacritty_terminal` and keep the gpui view in the same repo (chosen).** Everything about the terminal is in one place, so changing behavior or looks means touching one repo. Cost: the fork mixes upstream code with Portix code, so pulling upstream updates means a manual merge. `FORK.md` lists the patches to make that manageable.
2. Pure fork, with the view in `portix-ui`. The fork stays clean, but `portix-ui` would no longer be "no state, no I/O".
3. Pure fork, with the view in `portix-desktop`. Touches the fewest repos, but the view could not be reused anywhere else.

Zed's terminal crates were ruled out because they are GPL-3, depend on Zed internals, and are not published on crates.io.

## Design

### Repo `portix-terminal`

- Path `~/Documents/project/portix-terminal`, remote `github.com/chimeramutate/portix-terminal`. The first commit is the unmodified crates.io 0.26.0 source ("Fork alacritty_terminal 0.26.0 from crates.io as portix-terminal").
- Package `portix-terminal`, lib name `portix_terminal`. Upstream modules stay at the crate root, unchanged.
- `FORK.md` follows the `portix-gpui` format: where the code came from, plus a list of patches. The new `view` module is listed there as Portix's own code.
- New module `view`, which adds the dependency on `portix-ui`:
  - `TerminalView` (a gpui entity) owns the `Term`, the PTY, and alacritty's `EventLoop`. The event loop runs on its own thread. Events reach the entity through a channel, and the entity calls `cx.notify()`.
  - `TerminalView::spawn(program, args, font_size, window, cx)` starts the process in a PTY.
  - Rendering paints the visible grid row by row as styled text runs in a monospace font (Menlo), with a block cursor. ANSI colors map to a palette that is readable on both `portix-ui` themes.
  - Input turns key events into bytes (including arrows, ctrl combinations, and alt as ESC prefix) and writes them to the PTY. cmd-v pastes, using bracketed paste when the remote app turns it on.
  - The view converts its bounds into rows and columns, then resizes both the PTY and the `Term`.
  - Mouse: the wheel scrolls through scrollback, dragging selects text, cmd-c copies the selection.
  - When the process exits, the view shows "[process exited]" and keeps the last screen visible.
- Test: one check that spawns `printf hi` in a PTY and asserts that the grid contains `hi`. Upstream tests are kept.

### Integration in `portix-desktop`

- `profiles.rs` reads the extra fields as `Option` so profiles with missing fields still load.
- The command is `ssh -p <port> [-A if forwardAgent] [-J <jump user@host:port>] <username>@<host>`. Building it is a pure function with a unit test.
- `Workspace` gets a list of sessions (`profile name`, `Entity<TerminalView>`), the active view (Profiles or Terminals), and the active session.
- Enter or double-click on a card creates a session and switches to Terminals. The Terminals rail item is enabled whenever the session list is not empty. Its side panel lists the sessions. Each session has a close button that kills the process.
- RDP, SFTP, Kubernetes, and Kibana stay in `NOT_PORTED`.

## Security

### Threat model

There are three untrusted inputs:

1. **Bytes from the remote server.** A compromised server, or a malicious file shown with `cat`, can send any escape sequence.
2. **`profiles.json`.** Another app (Flutter) also writes this file, and it may be synced or tampered with. Its values must never become options or shell code.
3. **The clipboard during paste.** Text copied from a web page can hide control characters.

Protected assets: the local machine (no local command execution), the clipboard contents, the user's keystrokes, and the forwarded SSH agent.

### Possible holes and how each is closed

| # | Hole | Attack | Mitigation |
|---|------|--------|------------|
| S1 | Option injection in `ssh` | A host or username like `-oProxyCommand=sh -c ...` runs a local command | Validate `host`, `username`, and the jump host: not empty, no leading `-`, no whitespace, no control characters, a restricted character set (`[A-Za-z0-9._-]` for usernames, letters/digits/`.-:` and `[...]` for IPv6 hosts). Port must be 1-65535. Put `--` before the destination. Spawn with an argv array, never through a shell. Invalid profiles are refused with an error message. |
| S2 | Jump loop | `jumpProfileId` points to itself, or two profiles point to each other | Allow only one jump level, which is what `-J` supports. If the jump profile is itself, missing, or has its own jump, refuse it. |
| S3 | Clipboard read (OSC 52 `?`) | The server reads the local clipboard (passwords, tokens) | Ignore OSC 52 reads entirely. Never reply. |
| S4 | Clipboard write (OSC 52) | The server fills the clipboard with a malicious command, which the user then pastes somewhere else | Off in v1. Ignore it. |
| S5 | Reply injection | Sequences that make the terminal "answer" (title report `CSI 21 t`, DECRQSS, and similar) send text that the shell runs as input | Answer only the safe, fixed-format queries: DA, DSR cursor position, and color queries. Never echo data that came from the server (titles, strings). |
| S6 | Window manipulation (`CSI ... t`) | The server resizes, moves, or minimizes the window, or makes it enormous | Ignore every window op. Size comes only from the view's bounds. |
| S7 | Bracketed paste escape | Pasted text contains `ESC[201~`, which ends paste mode early, so the rest runs as commands | Strip `ESC` (0x1b) and every C0/C1 control character except `\t`, `\n`, `\r` from pasted text before writing it. |
| S8 | Multi-line paste without bracketed paste | A newline in the pasted text runs a command immediately | If bracketed paste is off and the text contains a newline, ask for confirmation first. The confirmation shows a preview. |
| S9 | UI spoofing through the title (OSC 0/2) | The remote title impersonates a different session or Portix UI | The session list always shows the profile name. The remote title is not shown in v1. |
| S10 | Malicious links (OSC 8) | Clicking opens `file://` or a custom URL scheme | Links are not opened in v1. They render as plain text. |
| S11 | Memory and CPU DoS | Endless output, huge resize requests, very long OSC strings | Cap scrollback at 10,000 lines. Clamp the grid to at least 1x1 and at most 1000x500. The PTY thread reads in fixed-size chunks. Rendering is throttled to one frame per vsync. |
| S12 | Orphaned processes | Closing a tab leaves `ssh` running with the agent still forwarded | Dropping a `TerminalView` sends SIGHUP to the child's process group, then SIGKILL after 2 s if it is still alive. Quitting the app closes every session. |
| S13 | Secret leaks | Passwords or output end up in logs or on disk | PTY data is never logged and scrollback is never written to disk. Passwords never go through argv or env (this matters later for Keychain autofill). |
| S14 | Unsafe code in the view | Memory bugs in our own code | Put `#![forbid(unsafe_code)]` in the `view` module. `unsafe` stays only in the upstream `tty` code. |
| S15 | Parser panic | Random bytes crash the whole app | Fuzz the parser and `Term` (below). A panic on the PTY thread ends that session only. |
| S16 | Supply chain | Malicious or vulnerable dependencies | Pin every git dependency by rev. Commit `Cargo.lock`. Run `cargo audit` as part of the checks. |

`StrictHostKeyChecking` is never set to `no`. Host-key verification stays with `ssh` and `~/.ssh/known_hosts`.

### Security tests

Every row above has at least one test that fails if the hole reopens.

`portix-desktop` (pure function `ssh_command`):

- S1: host `-oProxyCommand=x`, username `-x`, host with a space, `\n`, `;`, or `$(...)`, port 0, port 70000. All are refused.
- S1: a valid profile produces argv that ends with `--`, `user@host`, and contains no element that starts with `-o`.
- S2: a jump to itself, a missing jump, and a jump chained to another jump are all refused. A valid jump produces `-J user@jump:port`.

`portix-terminal` (feed bytes into `Term` and check what is written back to the PTY or the clipboard):

- S3/S4: `ESC]52;c;?BEL` and `ESC]52;c;aGk=BEL` produce no reply and no clipboard event.
- S5: set the title to `$(rm -rf ~)`, then send `CSI 21 t`. Nothing is written back to the PTY. `CSI c` (DA) still gets a reply.
- S6: `CSI 8;9999;9999 t` leaves the grid size unchanged.
- S7: pasting `a\x1b[201~rm -rf ~\n` in bracketed mode writes `ESC[200~` + `a[201~rm -rf ~\n` + `ESC[201~`, with no stray ESC.
- S8: multi-line text with bracketed paste off asks for confirmation and writes nothing yet.
- S11: resize to 0x0 becomes 1x1, resize to 100000x100000 becomes the maximum, and after 20,000 lines of output scrollback is ≤ 10,000.
- S12: spawn `sleep 100`, drop the view, and within 3 s the pid no longer exists.
- S15: 10,000 iterations of random bytes (seeded xorshift, no new dependency), with the grid resized at random, never panic.

## Calls left to the builder

- How grid painting works internally (for example caching shaped lines), as long as typing and output stay smooth.
- The exact ANSI palette values.
- Key mapping details beyond those listed above.
- What happens to the selected session when one is closed.
