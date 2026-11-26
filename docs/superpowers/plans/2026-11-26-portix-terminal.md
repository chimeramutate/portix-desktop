# portix-terminal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Embed a hardened SSH terminal in Portix, built on `portix-terminal`, a fork of `alacritty_terminal` 0.26.0 that Portix owns.

**Architecture:** Upstream modules stay at the crate root of `portix-terminal`. Portix's own code goes in `src/portix/`: reply filtering, paste sanitizing, key encoding, the PTY session, and the gpui `TerminalView`. Every security rule is a pure function with a unit test, so the view only wires them together. `portix-desktop` builds the `ssh` argv from a profile, validates it, and hosts the sessions.

**Tech Stack:** Rust 2024, `portix-terminal` (with alacritty's `vte`/`polling`/`libc`), `portix-ui` → `portix-gpui`.

**Spec:** `docs/superpowers/specs/2026-11-26-portix-terminal-design.md` (in portix-desktop). Read its Security section: S1–S16 are referenced by number below.

## Global Constraints

- Repo `~/Documents/project/portix-terminal`, remote `https://github.com/chimeramutate/portix-terminal`. Package `portix-terminal`, lib name `portix_terminal`, license Apache-2.0, `publish = false`.
- The first commit is the unmodified crates.io `alacritty_terminal` 0.26.0 source. Every later change to upstream files is listed in `FORK.md`.
- The `portix-ui` dependency is exactly `portix-ui = { git = "https://github.com/chimeramutate/portix-ui", rev = "0879a2b355b9a3238e2e58eeb78a06e23cdf0439" }`, the same rev in both `portix-terminal` and `portix-desktop`.
- No code copied from Zed's `terminal` / `terminal_view` crates (GPL-3).
- Scrollback is 10,000 lines. The grid is clamped to 1x1 … 1000 columns x 500 lines.
- PTY data is never logged and never written to disk. Passwords never go through argv or env.
- `ssh` is spawned with an argv array, never through a shell. `StrictHostKeyChecking` is never touched.
- The `src/portix/view.rs` module starts with `#![forbid(unsafe_code)]`.
- No new crates.io dependencies beyond what upstream already uses, plus `portix-ui`.

## Review Focus

1. **Wide characters (CJK, emoji) in the grid:** a double-width cell must take two columns, and the spacer cell after it must not be drawn. Otherwise everything after it on the line shifts. → Task 6 test `render_line_skips_wide_spacers`.
2. **IPv6 hosts:** `::1` as destination becomes `user@::1`, and as a jump it becomes `user@[::1]:22`. → Task 7 test `ipv6_host_and_jump`.
3. **Remote side ends** (network drop, `exit`): the session reports the exit, shows "[process exited]", and can still be closed. → Task 5 test `child_exit_is_reported`.
4. **Output flood** (`yes`, `cat` of a large file): the window stays responsive, and output is drawn at most once per frame. → Task 6 manual check step.
5. **Quitting Portix with sessions open:** no `ssh` processes are left behind. → Task 8 manual check step (`pgrep -f "ssh .*@"` after quit is empty).

---

### Task 1: Fork the crate

**Files:**
- Create: `~/Documents/project/portix-terminal/` (contents of `alacritty_terminal-0.26.0.crate`)
- Modify: `Cargo.toml` (package name, `[lib] name`, description, `publish = false`, `[package.metadata] upstream`)
- Modify: `tests/ref.rs` (`alacritty_terminal::` → `portix_terminal::`)
- Create: `FORK.md`, `README.md`

- [ ] **Step 1:** Download `https://static.crates.io/crates/alacritty_terminal/alacritty_terminal-0.26.0.crate`, extract it to `~/Documents/project/portix-terminal`, delete `.cargo_vcs_info.json` and `Cargo.toml.orig`, run `git init`, and make the first commit with everything: `Fork alacritty_terminal 0.26.0 from crates.io as portix-terminal`.
- [ ] **Step 2:** Rename: package `portix-terminal`, `[lib] name = "portix_terminal"`, and fix the crate name in `tests/ref.rs` and any doc tests.
- [ ] **Step 3:** Write `FORK.md` in the same format as portix-gpui's: origin, the lib rename, and an empty "Patches" list.
- [ ] **Step 4:** Run `cargo test`. Expected: all upstream tests (including `ref`) PASS.
- [ ] **Step 5:** Commit `Rename to portix-terminal`.

### Task 2: OSC 52 off by default (S3, S4)

**Files:**
- Modify: `src/term/mod.rs` (`Osc52`: move `#[default]` to `Disabled`)
- Create: `tests/security.rs`
- Modify: `FORK.md` (patch list)

**Interfaces:**
- Produces: `tests/security.rs` with helper `fn feed(term: &mut Term<Recorder>, bytes: &[u8])` (uses `vte::ansi::Processor`) and `struct Recorder(Rc<RefCell<Vec<Event>>>)` that implements `EventListener`. Task 3 reuses both.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn osc52_is_ignored_by_default() {
    let (mut term, events) = term_with_recorder(80, 24); // Config::default()
    feed(&mut term, b"\x1b]52;c;?\x07\x1b]52;c;aGk=\x07");
    assert!(!events.borrow().iter().any(|e| matches!(e, Event::ClipboardLoad(..) | Event::ClipboardStore(..))));
}
```

- [ ] **Step 2:** Run `cargo test --test security`. Expected: FAIL (a `ClipboardStore` event is emitted).
- [ ] **Step 3:** Move `#[default]` to `Osc52::Disabled`, update its doc comment, and add the patch line to `FORK.md`.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit `Disable OSC 52 by default`.

### Task 3: Reply filter (S5, S6, S9, S10, S15)

**Files:**
- Create: `src/portix/mod.rs`, `src/portix/reply.rs`
- Modify: `src/lib.rs` (`pub mod portix;`)
- Modify: `tests/security.rs`

**Interfaces:**
- Produces: `pub fn reply_for(event: &Event, size: WindowSize) -> Option<String>` in `portix::reply`. Fixed-format replies only: `PtyWrite(s)` → `Some(s)` (Term builds these itself, never from server strings), `TextAreaSizeRequest(f)` → `Some(f(size))`, `ColorRequest(i, f)` → `Some(f(portix::reply::ansi_rgb(i)))`. Everything else (`Title`, `ClipboardLoad`, `ClipboardStore`, `Bell`, …) → `None`.
- Produces: `pub fn ansi_rgb(index: usize) -> Rgb`: 16 ANSI colors, the 6x6x6 cube, the grayscale ramp, then foreground/background for indices 256/257 (`NamedColor::Foreground/Background`).
- Produces: `pub const SCROLLBACK: usize = 10_000;` and `pub fn config() -> Config` (default config with `scrolling_history: SCROLLBACK`, `osc52: Osc52::Disabled`).

- [ ] **Step 1: Failing tests** in `tests/security.rs`, where each test feeds bytes and then maps the recorded events through `reply_for`:
  - `title_is_never_echoed`: feed `ESC]2;$(rm -rf ~)BEL` + `CSI 21 t` + `CSI 20 t`. Assert that no reply contains `rm`.
  - `device_attributes_still_answered`: feed `CSI c`. Assert one reply that starts with `\x1b[?`.
  - `window_ops_do_not_resize`: feed `CSI 8;9999;9999 t`. Assert `term.columns() == 80 && term.screen_lines() == 24`.
  - `hyperlinks_never_emit_events`: feed `ESC]8;;file:///etc/passwdBEL x ESC]8;;BEL`. Assert that every recorded event maps to `None` except `Wakeup`/`MouseCursorDirty`.
  - `scrollback_is_capped`: `Term::new(config(), ..)`, feed 20,000 lines of `x\r\n`. Assert `term.grid().history_size() <= SCROLLBACK`.
  - `random_bytes_never_panic`: xorshift64 seeded with `0x9E3779B97F4A7C15`, 10,000 iterations of 1–512 random bytes. Every 100 iterations, resize to a random `TermSize` in 1..=200 x 1..=100. The only assertion is that the loop completes.
- [ ] **Step 2:** Run `cargo test --test security`. Expected: FAIL (`portix::reply` missing).
- [ ] **Step 3:** Implement `reply.rs` as described in Interfaces.
- [ ] **Step 4:** Run `cargo test --test security`. Expected: PASS. If a test exposes an upstream leak, patch upstream, list it in `FORK.md`, and re-run.
- [ ] **Step 5:** Commit `Add reply filter and security tests`.

### Task 4: Paste and keys (S7, S8)

**Files:**
- Create: `src/portix/paste.rs`, `src/portix/keys.rs` (unit tests inline)

**Interfaces:**
- Produces: `pub enum Paste { Write(Vec<u8>), Confirm(String) }` and `pub fn paste(text: &str, bracketed: bool, confirmed: bool) -> Paste`. Sanitize by dropping `\x1b` and every C0 (except `\t`, `\n`, `\r`) and C1 (U+0080–U+009F) character, then turn `\r\n` and `\n` into `\r`. Bracketed: wrap in `\x1b[200~`…`\x1b[201~`. Not bracketed, a newline in the text, and `!confirmed` → `Confirm(sanitized)`.
- Produces: `pub fn key_bytes(key: &str, key_char: Option<&str>, ctrl: bool, alt: bool, shift: bool, app_cursor: bool) -> Option<Vec<u8>>`. Takes plain parameters, not gpui types, so this module stays gpui-free and testable. Use gpui key names (`"up"`, `"enter"`, `"backspace"`, `"escape"`, `"tab"`, `"home"`, `"end"`, `"pageup"`, `"pagedown"`, `"delete"`, `"f1"`–`"f12"`).

- [ ] **Step 1: Failing tests**
  - `paste_strips_bracket_terminator`: `paste("a\x1b[201~rm -rf ~\n", true, false)` == `Write(b"\x1b[200~a[201~rm -rf ~\r\x1b[201~")`
  - `paste_strips_c0_c1`: `paste("a\x03b\u{9b}c\td", true, false)` contains `abc\td` between the markers.
  - `multiline_unbracketed_needs_confirm`: `paste("ls\nrm x\n", false, false)` == `Confirm("ls\rrm x\r")`. With `confirmed = true` it is `Write(b"ls\rrm x\r")`.
  - `single_line_unbracketed_writes`: `paste("ls", false, false)` == `Write(b"ls")`.
  - `keys`: `enter`→`\r`, `backspace`→`\x7f`, ctrl+`c`→`\x03`, alt+`x`→`\x1bx`, `up`→`\x1b[A`, `up` with app_cursor→`\x1bOA`, plain `a` with key_char `a`→`a`, `f5`→`\x1b[15~`.
- [ ] **Step 2:** Run `cargo test portix::`. Expected: FAIL.
- [ ] **Step 3:** Implement `paste` and `key_bytes`.
- [ ] **Step 4:** Run `cargo test portix::`. Expected: PASS.
- [ ] **Step 5:** Commit `Add paste sanitizing and key encoding`.

### Task 5: PTY session (S11, S12, S13)

**Files:**
- Create: `src/portix/session.rs` (unit tests inline)

**Interfaces:**
- Consumes: `reply::config()`, `reply::reply_for`.
- Produces:
  - `pub const MAX_COLS: usize = 1000; pub const MAX_LINES: usize = 500;` and `pub fn clamp_size(cols: usize, lines: usize) -> TermSize` (each clamped to `1..=MAX`).
  - `pub struct Session` with:
    - `pub fn spawn(program: &str, args: &[String], cols: usize, lines: usize) -> io::Result<(Session, std::sync::mpsc::Receiver<Event>)>`
    - `pub fn term(&self) -> &Arc<FairMutex<Term<Proxy>>>`
    - `pub fn write(&self, bytes: Vec<u8>)`
    - `pub fn resize(&self, cols: usize, lines: usize, cell_w: u16, cell_h: u16)`
    - `pub fn pid(&self) -> u32`
  - `pub struct Proxy`: an `EventListener` that, for every event, writes `reply_for(..)` to the PTY through the `Notifier` when it is `Some`, then forwards the event on the channel. It holds the current `WindowSize` in an `Arc<Mutex<_>>` that `resize` updates.
  - Env: `TERM=xterm-256color`, `COLORTERM=truecolor`. No logging of bytes.
  - `Drop`: send `Msg::Shutdown`, `kill(-pid, SIGHUP)` (alacritty's child calls `setsid`, so pid == pgid). Then a detached thread sleeps 2 s and sends `SIGKILL` to the group if `kill(pid, 0)` still succeeds. Keep the `unsafe` in this file only, one call per line, with a `// SAFETY:` comment.

- [ ] **Step 1: Failing tests**
  - `spawns_and_renders`: `spawn("/bin/sh", &["-c".into(), "printf hi".into()], 80, 24)`, wait for `Event::ChildExit` (timeout 5 s), assert row 0 of the grid starts with `hi`.
  - `child_exit_is_reported`: `sh -c 'exit 3'`. Receive `ChildExit(status)` with `status.code() == Some(3)` within 5 s.
  - `drop_kills_process_group`: `spawn("/bin/sleep", &["100".into()], 80, 24)`, record `pid`, drop. Within 3 s `kill(pid, 0)` fails.
  - `clamp_size_bounds`: `(0,0)`→`1x1`, `(100000,100000)`→`1000x500`, `(80,24)`→`80x24`.
  - `size_report_uses_view_size`: spawn `sh -c 'printf "\033[18t"; sleep 1'`, resize to 100x30 first. No panic, and the session's own reply path is exercised (assert the `TextAreaSizeRequest`/`PtyWrite` event arrives on the channel).
- [ ] **Step 2:** Run `cargo test portix::session`. Expected: FAIL.
- [ ] **Step 3:** Implement with `tty::new(&Options { shell: Some(Shell::new(..)), env, .. }, window_size, 0)`, `EventLoop::new(term.clone(), proxy, pty, false, false)?.spawn()`, and `Notifier(loop.channel())`.
- [ ] **Step 4:** Run `cargo test`. Expected: all PASS.
- [ ] **Step 5:** Commit `Add PTY session with group kill on drop`.

### Task 6: TerminalView (gpui)

**Files:**
- Create: `src/portix/view.rs`, with `#![forbid(unsafe_code)]` as its first line
- Modify: `Cargo.toml` (add `portix-ui`, rev from Global Constraints)
- Create: `examples/term.rs` (opens one window running `$SHELL`, for manual checks)
- Modify: `README.md`

**Interfaces:**
- Consumes: `Session`, `paste::paste`, `keys::key_bytes`, `clamp_size`.
- Produces:
  - `pub struct TerminalView`, `impl Render`, `impl Focusable`.
  - `pub fn new(program: &str, args: &[String], font_size: f32, window: &mut Window, cx: &mut Context<Self>) -> io::Result<Self>`
  - `pub fn exited(&self) -> bool`
  - `pub fn render_line(cells: &[(char, CellFlags, Hsla /*fg*/, Hsla /*bg*/)]) -> (String, Vec<(usize /*byte len*/, Hsla, Hsla)>)`: a pure helper that skips `WIDE_CHAR_SPACER` cells and merges same-styled neighbors into runs.
- Behavior:
  - A background task drains the `Receiver<Event>`, coalesces events, and calls `cx.notify()` at most once per frame. `ChildExit` sets `exited`.
  - Render: measure the cell size from `window.text_system()` for Menlo at `font_size`. Compute cols/lines from the bounds through `canvas` prepaint. Call `session.resize` on change. Each visible line becomes a `StyledText::new(text).with_runs(runs)`. Colors come from `reply::ansi_rgb`. Draw a block cursor. When exited, add a "[process exited]" line.
  - Input: `on_key_down` → `key_bytes` → `session.write`. cmd-v → `paste(text, mode.contains(BRACKETED_PASTE), false)`. `Confirm` shows an inline bar with a preview (first 3 lines) and Paste/Cancel buttons. Paste calls `paste(.., true)`. cmd-c → `term.selection_to_string()` → `cx.write_to_clipboard`.
  - Mouse: wheel → `term.scroll_display(Scroll::Delta(lines))`, down/drag/up → `Selection::new(SelectionType::Simple, ..)` / `update`.
  - Window title events and hyperlinks are ignored (S9, S10).

- [ ] **Step 1: Failing test** `render_line_skips_wide_spacers`: input `[('漢', WIDE_CHAR, A, B), (' ', WIDE_CHAR_SPACER, A, B), ('x', empty, C, B)]` → text `"漢x"`, runs `[(3, A, B), (1, C, B)]`.
- [ ] **Step 2:** Run `cargo test portix::view`. Expected: FAIL.
- [ ] **Step 3:** Implement `render_line`, then the view.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS. Run `cargo build --example term`. Expected: builds without warnings.
- [ ] **Step 5: Manual check:** run `cargo run --example term` and verify each item: typing `ls` works, `vim` works with arrows, the window resizes and `tput cols` follows, scrollback scrolls, selecting + cmd-c copies, pasting a 2-line text asks for confirmation, `printf '\e]52;c;aGk=\a'` leaves the clipboard unchanged, `yes | head -n 2000000` keeps the window responsive, and `printf '漢x\n'` aligns.
- [ ] **Step 6:** Run `cargo install cargo-audit --locked` if it is missing, then `cargo audit`. Expected: no vulnerabilities (S16). Record any warning in `README.md`.
- [ ] **Step 7:** Commit `Add gpui TerminalView`.
- [ ] **Step 8:** Ask the user before creating the GitHub repo `chimeramutate/portix-terminal` and pushing `main`. Record the pushed commit hash for Task 8.

### Task 7: ssh command from a profile (S1, S2)

**Files:**
- Modify: `portix-desktop/src/profiles.rs`
- Create: `portix-desktop/src/ssh.rs` (unit tests inline)

**Interfaces:**
- Produces in `profiles.rs`: `Profile` gains `#[serde(default)] pub id: String, pub host: String, pub port: Option<u32>, pub username: String, pub jump_profile_id: Option<String>, pub forward_agent: bool, pub terminal_font_size: Option<f32>`, with `#[serde(rename_all = "camelCase")]` on the struct. Existing `name`/`group`/`color` are unchanged. The port is `u32` rather than `u16` so that one bad port fails only that profile in `ssh.rs`, not the load of the whole file.
- Produces in `ssh.rs`: `pub fn ssh_command(profile: &Profile, all: &[Profile]) -> Result<Vec<String>, String>`, which returns argv **without** the program (`ssh`). Output shape: `["-p", port, ("-A")?, ("-J", "user@jhost:jport")?, "--", "user@host"]`. Port defaults to 22.
  - `valid_user`: non-empty, `[A-Za-z0-9._-]`, does not start with `-`.
  - `valid_host`: non-empty, does not start with `-`, and is either `[A-Za-z0-9.-]` or an IPv6 address (`[0-9A-Fa-f:.]` containing `:`).
  - Port: 1..=65535.
  - Jump: looked up by `id` in `all`. Refused if missing, if it is the profile itself, or if the jump profile has its own `jump_profile_id`. The jump's host/user/port go through the same validation. An IPv6 jump host is written as `user@[addr]:port`.
  - Error text names the field, for example `"invalid host"`, and never echoes the value (it may contain control characters).

- [ ] **Step 1: Failing tests**
  - `valid_profile`: `{user:"ops", host:"10.0.0.1", port:2222}` → `["-p","2222","--","ops@10.0.0.1"]`.
  - `forward_agent_adds_a`: contains `"-A"` before `"--"`.
  - `rejects_option_injection`: host `-oProxyCommand=x`, user `-x`, host `a b`, host `a\nb`, host `a;b`, host `$(id)`, user `a@b` → all `Err`.
  - `rejects_bad_ports`: 0 and 70000 → `Err`.
  - `no_option_after_separator`: for the valid case, nothing after `"--"` starts with `-`.
  - `jump_rules`: a valid jump gives `["-p","22","-J","j@10.0.0.9:22","--","ops@10.0.0.1"]`. A self jump, a missing id, and a chained jump → `Err`.
  - `ipv6_host_and_jump`: host `::1` → ends with `"ops@::1"`. Jump host `fe80::1` → `"-J","j@[fe80::1]:22"`.
  - `loads_real_shape`: the existing `profiles::load` test JSON plus `"username"`, `"jumpProfileId": null`, `"forwardAgent": false` still loads.
- [ ] **Step 2:** Run `cargo test`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test`. Expected: PASS.
- [ ] **Step 5:** Commit `Build validated ssh argv from profiles`.

### Task 8: Terminals in the workspace

**Files:**
- Modify: `portix-desktop/Cargo.toml` (add `portix-terminal = { git = "https://github.com/chimeramutate/portix-terminal", rev = "<hash from Task 6 Step 8>" }`)
- Modify: `portix-desktop/src/main.rs`

**Interfaces:**
- Consumes: `ssh::ssh_command`, `portix_terminal::portix::view::TerminalView::new`.
- Produces in `main.rs`: `enum View { Profiles, Terminals }`, `struct TermSession { name: String, view: Entity<TerminalView> }`, and `Workspace` fields `view: View`, `sessions: Vec<TermSession>`, `active_session: usize`, `open_error: Option<String>`.

- [ ] **Step 1:** Add action `Open`, bound to `enter` in the `ProfileGrid` context. Double-click on a card (`event.click_count == 2`) does the same. `fn open_selected(&mut self, window, cx)`: run `ssh_command`. On `Err(e)`, set `open_error = Some(format!("{}: {e}", profile.name))` and show it with `ui::message`. On `Ok(args)`, call `TerminalView::new("/usr/bin/ssh", &args, profile.terminal_font_size.unwrap_or(13.), ..)`, push a session, set `view = Terminals`, and focus it.
- [ ] **Step 2:** Rail: remove `Terminals` from `NOT_PORTED`. Show it as its own `rail_item` that is enabled when `!sessions.is_empty()` and active when `view == Terminals`. The Profiles item sets `view = Profiles` (and keeps its collapse toggle when already active).
- [ ] **Step 3:** In the Terminals view, the side panel lists sessions with `ui::side_panel_tile` showing the profile name only (S9), plus a close button. Closing drops the entity, which kills the group (S12). The main area renders the active `TerminalView`.
- [ ] **Step 4:** Run `cargo build`. Expected: builds without warnings. Run `cargo test`. Expected: PASS.
- [ ] **Step 5: Manual check:** run `cargo run` and verify each item: Enter on a profile opens a terminal with the ssh password prompt, the profile grid is still reachable from the rail, two sessions can be switched, closing a session ends its `ssh` (`pgrep -f "ssh .*@"`), Cmd-Q with sessions open leaves no `ssh`, and a profile edited to host `-oProxyCommand=x` shows "invalid host" and spawns nothing.
- [ ] **Step 6:** Commit `Open SSH profiles in embedded terminals`.
