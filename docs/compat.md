# Servo compat

What works and what doesn't when Zervo renders Zzz's production web app, and
how to recheck it. Until the gaps below close, Zzz is used in a browser.

Results are tied to the **Servo revision** they were observed at. The pinned
revision is the `servo` entry's `source` in `Cargo.lock`;
`scripts/servo_gap_sites.sh` prints it. Everything here was observed at
`fb7f6d0` on Debian, GNOME on Wayland with XWayland, at scale 1.0, unless the
entry says it's from source.

Commands run from this repo's root unless a step says otherwise, in fish or a
POSIX shell, and depend on no variable set earlier.

- [What works](#what-works)
- [Known gaps](#known-gaps)
- [Rebuilding](#rebuilding)
- [Bumping the Servo pin](#bumping-the-servo-pin)
- [The walk-through](#the-walk-through)

## What works

- **Hydration**: the app's JS runs, and the bootstrap and login forms render.
- **Bootstrap and login**, through the `Secure`-stripping proxy only (see
  [Secure cookies](#secure-cookies-are-dropped-on-httplocalhost)).
- **WebSocket**: connects, and reconnects on its own after a daemon restart.
- **Terminal**: xterm.js renders, single words typed at the prompt arrive, and
  text copied in the terminal pastes into other apps.
- **Text fields**: typing, selection inside the field, Ctrl+C and Ctrl+V.
- **Pointer**: clicking and wheel scrolling.
- **Focus**: typing still works after alt-tabbing away and back.
- **Idle**: an untouched login form leaves the process near idle, woken only
  by the caret blink. A visible window repaints at once when restored from
  minimized and shows terminal output as it arrives.
- **Close**: closing the window exits with status 0.

**Not yet checked**: capitals, symbols, Ctrl+C, and key repeat in the
terminal; IME; `<select>` and dialogs; the system theme; Zzz's own context
menus; copy buttons; idle CPU with the window visible.

## Known gaps

Each entry gives what you see, why, who owns the fix, and how to recheck.
Servo paths are relative to the cached checkout,
`~/.cargo/git/checkouts/servo-*/<rev>/`, and line numbers hold only at the
revision named. The owner is Servo upstream, Zervo, the Zzz frontend, or Fuz's
auth crates (`fuz_auth`).

### Secure cookies are dropped on `http://localhost`

**Blocks login**: nothing behind the auth gate is reachable without the
workaround.

- **Seen**: bootstrap and login succeed on the server, and the page shows the
  login form again.
- **Cause**: Zzz's session cookie is `HttpOnly; Secure; SameSite=Strict`.
  Servo ignores a `Secure` cookie unless the URL's scheme is secure, and
  `ServoUrl::is_secure_scheme` (`components/url/lib.rs`) is only `https` and
  `wss`, with no exemption for loopback, which Firefox and Chromium both make.
  The cookie sites that call it, at `fb7f6d0`: `components/net/cookie.rs:206`
  and `:374`, `components/net/cookie_storage.rs:57` and `:157`.
- **Owner**: Servo upstream. The fix is a cookie-specific "potentially
  trustworthy" check at those sites
  (`ImmutableOrigin::is_potentially_trustworthy` in
  `components/url/origin.rs` already covers `localhost`, `*.localhost`, and
  loopback addresses). Alternatively `fuz_auth` could omit `Secure`
  when the daemon serves plain http on loopback.
- **Workaround**: point Zervo at `scripts/strip_secure_proxy.py` (started by
  `scripts/audit_daemon.sh`), which removes `Secure` from `Set-Cookie`.
- **Recheck**: `scripts/servo_gap_sites.sh`; fixed when those sites stop
  calling `is_secure_scheme()`. Then log in with Zervo pointed at the daemon
  directly ([walk-through](#the-walk-through) step 5). If that works, delete
  the proxy.

### `keypress.charCode` is always 0, so space is dead in the terminal

- **Seen**: in an xterm.js terminal, typing a space does nothing. The app's
  own input box that sends to the terminal works.
- **Cause**: Servo builds every platform `KeyboardEvent` with a literal
  `0, /* char_code */` (`components/script/dom/event/keyboardevent.rs:101`;
  the `keypress` is created at
  `components/script/dom/document/document_event_handler.rs:1434`). xterm.js
  takes a character from `keydown` only when its `keyCode` is 48 or more and
  it isn't `A`–`Z`; everything else it reads from `keypress.charCode`. So by
  source the dead set is space, `A`–`Z`, and every character Servo gives
  `keyCode` 0: the shifted digits `! @ # $ % ^ & * ( )` and anything
  non-ASCII. Lowercase letters, digits, and the other punctuation (shifted or
  not) are sent on `keydown` and work. Only space has been observed. Zervo
  forwards the right key.
- **Owner**: Servo upstream (set `charCode` on `keypress` from the key's
  character). The Zzz frontend could work around it with a
  `term.attachCustomKeyEventHandler` that feeds `ev.key` to the terminal on a
  `keypress` whose `charCode` is 0, which is inert in real browsers.
- **Recheck**: `scripts/servo_gap_sites.sh`; fixed when the literal `0` is
  gone. Live: type `echo Hello, World! $HOME` in a terminal. The spaces, `H`,
  `W`, `!`, and `$HOME` go missing while the gap is present.

### Page text can't be selected

- **Seen**: dragging over page text selects nothing. Selection inside inputs
  and textareas works.
- **Cause**: unimplemented in Servo. Only text controls get a selection
  (`Node::selection` in `components/script/dom/node/node.rs:2274`), there's no
  mouse-drag selection in `document_event_handler.rs`, and `user-select` is
  behind stylo's `layout.unimplemented` pref
  (`components/script/dom/selection.rs:277` has the TODO).
- **Owner**: Servo upstream, a large feature. The app's copy buttons mitigate
  it once [`navigator.clipboard`](#no-dom-clipboard-events-or-navigatorclipboard)
  is on.
- **Recheck**: `scripts/servo_gap_sites.sh`; live, drag across a paragraph on
  the about page.

### Paste into the terminal does nothing

- **Seen**: neither Ctrl+V nor Ctrl+Shift+V pastes into an xterm.js terminal.
- **Cause**: xterm.js takes paste only from a DOM `paste` event. Ctrl+V is
  `^V` to a terminal in any browser, and Zervo forwards Ctrl+Shift+V as a
  plain key.
- **Owner**: Zervo. Send `InputEvent::EditingAction` with
  `EditingActionEvent::Paste` for Ctrl+Shift+V instead of the key (optionally
  `Copy` for Ctrl+Shift+C), leaving plain Ctrl+X/C/V to the page. servoshell
  does this in `ports/servoshell/desktop/headed_window.rs`. The action can't
  be a follow-up to an unhandled keydown: Servo reports a character key's
  result from the `keypress` after it, so a text field that already pasted
  wouldn't come back `Consumed` and would paste twice.
- **Recheck**: copy text in another app, focus a terminal, press
  Ctrl+Shift+V.

### No DOM clipboard events or `navigator.clipboard`

From source; not yet observed.

- **Seen**: the page's `copy`, `cut`, and `paste` handlers never run, and
  `navigator.clipboard` is `undefined`, so copy buttons do nothing. Text
  fields still cut, copy, and paste through Servo's own key handling.
- **Cause**: Servo fires the clipboard events only for an
  `InputEvent::EditingAction`, which Zervo doesn't send, and the async
  clipboard API is behind the `dom_async_clipboard_enabled` pref, default
  `false` (`components/config/prefs.rs`).
- **Owner**: Zervo: the editing actions above, and the pref turned on through
  `ServoBuilder::preferences`.
- **Recheck**: `scripts/servo_gap_sites.sh` for the pref's default; live, press
  a copy button in the app and paste elsewhere.

### Clipboard goes through X11

- **Seen**: nothing on a Wayland session with XWayland. Without XWayland
  (from source) the clipboard becomes an in-process string that works only
  inside the window.
- **Cause**: Servo's default clipboard delegate uses `arboard`, locked here
  with only its X11 backend.
- **Owner**: Zervo: a custom `ClipboardDelegate`
  (`components/servo/clipboard_delegate.rs`, set with
  `WebViewBuilder::clipboard_delegate`) over a Wayland clipboard, which needs
  a new dependency.
- **Recheck**: `scripts/servo_gap_sites.sh` prints the locked backends.

### The session doesn't survive a relaunch

Moot until the `Secure` cookie gap is fixed.

- **Seen**: every launch starts logged out.
- **Cause**: `Opts::config_dir` (`components/config/opts.rs`) defaults to
  `None`, so the cookie jar and local storage live in memory.
- **Owner**: Zervo: create a directory (mode `0700`; Servo panics if it can't
  write there) and pass it as `Opts::config_dir` to `ServoBuilder::opts`.
  Cookies are written only on a clean exit.
- **Recheck**: log in, close the window, launch again at the same URL.

### No context menu, and `<select>` doesn't open

A missing context menu is observed; the rest is from source.

- **Seen**: right-click shows nothing. Clicking a `<select>` leaves it closed,
  `confirm()` returns `false` and `prompt()` returns `null` at once, and a
  file input picks nothing.
- **Cause**: Servo hands these to the embedder through
  `WebViewDelegate::show_embedder_control`
  (`components/servo/webview_delegate.rs`), which Zervo doesn't implement, so
  each answers with its default.
- **Owner**: Zervo.
- **Not yet checked**: whether Servo fires the `contextmenu` event that opens
  Zzz's own in-page menus.
- **Recheck**: right-click the page and an item with Zzz's own menu (a chat,
  a file); open a `<select>` (the sort picker above a sortable list).

### No IME

From source; not yet observed.

- **Seen**: input-method composition does nothing. Plain typing is
  unaffected.
- **Cause**: winit delivers IME events only after `set_ime_allowed(true)`,
  which should follow Servo's `EmbedderControl::InputMethod` requests through
  the same unimplemented delegate method.
- **Owner**: Zervo.
- **Recheck**: with an input method active, compose into a text field.

### The theme doesn't follow the system

From source; not yet observed.

- **Seen**: `prefers-color-scheme` stays at Servo's default when the desktop
  switches between light and dark.
- **Cause**: Zervo forwards winit's `ThemeChanged`, and winit reports no theme
  on Wayland.
- **Owner**: Zervo: read the desktop portal's color-scheme setting.
- **Recheck**: with the window open, run
  `gsettings set org.gnome.desktop.interface color-scheme prefer-dark` (and
  `default` to restore) and watch a page that follows the system scheme.

### A hidden window isn't throttled

- **Seen**: with the window covered or on another workspace, a page running a
  CSS animation keeps waking the event loop many times a second, with no
  repaints. A page that isn't animating wakes only for the caret blink.
- **Cause**: Zervo stops painting a hidden window, but winit reports no
  `Occluded` event on Wayland, so Servo isn't told the page is hidden. With no
  paints, Servo's script thread falls back to its own rendering timer
  (`components/script/script_thread.rs`) and keeps producing frames.
- **Owner**: Zervo: needs an occlusion signal (the `xdg_toplevel` suspended
  state, which winit doesn't surface) to pass to the `WebView`.
- **Recheck**: load a page with an infinite CSS animation under
  `RUST_LOG=zervo=debug`, cover the window, and watch the `loop activity`
  lines. Fixed when they stop.

### Other unwired input

From source; not yet observed against the app.

- **Seen**: the page gets no window focus or blur events, dropping a file on
  the window does nothing, and dragging a selection past the window edge
  doesn't scroll.
- **Cause**: Zervo doesn't forward these. The `WebView` is focused once at
  creation, and moves outside the window during a drag are dropped.
- **Owner**: Zervo (file drops also need a Servo entry point).
- **Recheck**: drag-select inside a long textarea past the bottom edge of the
  window. Zzz uses neither file drops nor window focus events.

### Minor visual differences

Compare against a browser on the same URL and list each difference here
(page, element, what differs, a guess at the CSS feature):

- While the window edge is dragged, the content warps and lags, then settles
  correctly. Cause not traced.

### Not Servo gaps

- **Slow WebSocket reconnect** after a daemon restart: the frontend's
  exponential backoff, identical in a browser.
- **A slow second launch**: `cargo run` relinking the large debug binary. Run
  `./target/debug/zervo` directly.
- **The Vite dev server fails to load**: its cross-origin module graph fails
  Servo's CORS check. Always test Zzz's production build.

## Rebuilding

Servo compiles from source and needs a native toolchain, declared in
`zap.ts`:

```sh
zap apply            # preview what would be installed
zap apply --wetrun   # install it (apt)
cargo build -p zervo --locked
```

- The first build, and the first after a pin bump, compiles all of Servo
  (SpiderMonkey, WebRender, stylo). Later builds are incremental.
- While cargo fetches Servo it holds the global package-cache lock, which
  blocks cargo in every other checkout until the fetch finishes.
- `--locked` keeps `Cargo.lock`, and so the Servo revision, as committed.

## Bumping the Servo pin

`servo` is a git dependency on `branch = "main"`, and `Cargo.lock` holds the
revision. A bump is the only way to pick up an upstream fix.

1. Move the lock to Servo's current `main`:

   ```sh
   cargo update -p servo
   ```

2. Check the Rust toolchain. Run from this repo, not inside the Servo
   checkout, where rustup would install Servo's own pin:

   ```sh
   rustc --version
   scripts/servo_gap_sites.sh
   ```

   The script prints the channel Servo pins beside this repo's (if it finds no
   cached checkout yet, start step 3 and come back). Keep this repo's
   `rust-toolchain.toml` at or above Servo's. An older toolchain can still
   build down to the `rust-version` in Servo's root `Cargo.toml`, but Servo
   tests only its pinned channel: a compile error inside a Servo crate naming
   a missing or unstable std feature means move this repo's pin up.

3. Rebuild with `cargo build -p zervo --locked`. Compile errors in this repo's
   crate mean the embedding API moved. Afterwards this should print nothing; a
   line means two `raw-window-handle` versions, so the handles Zervo passes no
   longer unify with Servo's:

   ```sh
   cargo tree -d -p zervo | grep raw-window-handle
   ```

4. Reconcile `crates/zervo/src/` against the new checkout, reading the
   symbols there: the embedding API in `components/servo/` (`servo.rs`,
   `webview.rs`, `webview_delegate.rs`), the input types in
   `components/shared/embedder/input_events.rs`, and servoshell's
   `ports/servoshell/desktop/headed_window.rs` and `desktop/keyutils.rs`,
   which `input.rs` and `keyutils.rs` mirror.
5. Reconcile `zap.ts` against Servo's apt list
   (`python/servo/platform/linux_packages/apt/apt_common.txt` in the
   checkout). `scripts/servo_gap_sites.sh` lists any package the list has and
   `zap.ts` lacks; `libfreetype6-dev` is a deliberate rename. Then
   `zap apply`, and `zap apply --wetrun` if it shows changes.
6. Run the gates. Clippy must print no warnings:

   ```sh
   cargo build -p zervo --locked
   cargo clippy -p zervo --all-targets --locked
   cargo test -p zervo --locked
   ```

7. Check the upstream gaps with `scripts/servo_gap_sites.sh`, which prints
   each one's source sites and what "still present" looks like.
8. Do [the walk-through](#the-walk-through), then update this doc: the
   revision observed at, each gap's line numbers, and delete the gaps that
   are fixed.

To abandon a bump, restore `Cargo.lock` from git; the old revision's checkout
and build artifacts are still cached.

## The walk-through

Needs Postgres running locally with a role for your user (`createdb` works), a
`zzz` checkout at `../zzz` beside this repo, buildable per its own README with
`npm install` done, and a graphical session.

1. **Build Zzz's daemon and production app.** In the `zzz` checkout:

   ```sh
   cargo build -p zzz_server
   cargo xtask prod-setup
   gro build
   ```

   `prod-setup` creates Zzz's `.env.production` if it's missing, and
   `gro build` reads it and writes `build/`.

2. **Build Zervo**: `cargo build -p zervo --locked`.

3. **Start the audit daemon and the proxy.**

   ```sh
   scripts/audit_daemon.sh start
   ```

   Idempotent. It creates a state dir (`~/.local/state/zervo_audit`, or under
   `$XDG_STATE_HOME`) holding a generated cookie key, the daemon's working
   directory, logs, and pid files; creates the `zervo_audit` database if it's
   missing; and, while no account exists, prints a bootstrap token. `zzzd`
   listens on `4480`, and the `Secure`-stripping proxy on `4481` forwards to
   it. `scripts/audit_daemon.sh status` shows all of this again, and the
   variables at the top of the script override each default.

4. **Launch Zervo through the proxy.**

   ```sh
   sh -c './target/debug/zervo http://localhost:4481/; echo "exit status: $?"'
   ```

5. **Recheck the cookie gap** (any time after step 3) by launching at the
   daemon directly. Logging in here works only once Servo accepts `Secure`
   cookies on loopback.

   ```sh
   sh -c './target/debug/zervo http://localhost:4480/; echo "exit status: $?"'
   ```

6. **Checklist.** For each item, what working looks like → the gap a failure
   maps to.

   1. **Hydration**: the bootstrap form (first run) or login form appears →
      a regression. A blank page or raw HTML means the JS didn't run.
   2. **Bootstrap** (first run): enter the printed token, a username, and a
      password of twelve characters or more, and land in the app → through
      the proxy, a regression; direct, [Secure
      cookies](#secure-cookies-are-dropped-on-httplocalhost).
   3. **Login**: reload or relaunch, log in, land in the app → same as
      bootstrap.
   4. **WebSocket connects**: the app shows itself connected → a regression.
   5. **WebSocket reconnects**: run `scripts/audit_daemon.sh restart` from a
      second terminal; the app reconnects on its own within its backoff, or at
      once on a manual connect → a regression if it never does.
   6. **Terminal renders**: open a terminal page and start a shell; the
      prompt draws with the right font and colors → a regression.
   7. **Terminal typing**: type `echo Hello, World! $HOME` and Enter, and
      every character arrives →
      [`keypress.charCode`](#keypresscharcode-is-always-0-so-space-is-dead-in-the-terminal).
      Hold a lowercase key and it repeats → a regression in key forwarding.
   8. **Terminal Ctrl+C**: run `sleep 100`, press Ctrl+C, and the prompt
      returns → a regression in key forwarding.
   9. **Terminal copy**: select terminal text, copy, and paste into another
      app → [clipboard through X11](#clipboard-goes-through-x11).
   10. **Terminal paste**: copy text elsewhere and press Ctrl+Shift+V in the
       terminal → [paste](#paste-into-the-terminal-does-nothing).
   11. **Text-field clipboard**: Ctrl+A, Ctrl+C, Ctrl+V, and Ctrl+X behave in
       a text input, to and from other apps → a regression, or
       [clipboard through X11](#clipboard-goes-through-x11).
   12. **Copy buttons**: a copy button puts text on the clipboard →
       [`navigator.clipboard`](#no-dom-clipboard-events-or-navigatorclipboard).
   13. **Right-click**: a context menu appears on plain page content and on
       an item with Zzz's own menu → [no context
       menu](#no-context-menu-and-select-doesnt-open).
   14. **`<select>`**: a dropdown opens and a choice sticks → same entry.
   15. **Dialogs**: on the files page with a workspace open, "new file" asks
       for a name and creates the file; deleting a chat from Zzz's menu asks
       first → same entry (unhandled, the action silently does nothing).
   16. **IME**: composing into a text field shows and commits the
       composition → [no IME](#no-ime).
   17. **Page text selection**: drag across a paragraph and it highlights and
       copies → [text selection](#page-text-cant-be-selected).
   18. **Focus after alt-tab**: switch away and back, and typing goes to the
       field that had focus → a regression.
   19. **Visual parity**: open `http://localhost:4480/` in a browser beside
       Zervo and compare the same pages (about, chats, files, a terminal) →
       list under [visual differences](#minor-visual-differences).
   20. **Theme**: the page matches the desktop's light or dark setting →
       [theme](#the-theme-doesnt-follow-the-system).
   21. **Idle CPU**: with the window open and untouched, the second `zervo`
       line from `top -b -d 5 -n 2 | grep zervo` shows the process near idle,
       not holding a core → a regression in the render loop.
       `RUST_LOG=zervo=debug` logs the wake-ups, spins, and paints behind any
       load.
   22. **Session persistence**: close the window and run step 4 again, and
       you're still logged in →
       [session](#the-session-doesnt-survive-a-relaunch).
   23. **Clean close**: closing the window prints `exit status: 0`, with no
       `SIGSEGV` or panic above it → a regression in teardown order.

7. **Stop, and update this doc** with what moved.

   ```sh
   scripts/audit_daemon.sh stop
   ```

   The state dir and database are kept for the next walk. To start over with
   a fresh bootstrap, stop the daemon, drop the database with
   `dropdb zervo_audit`, and delete the state dir that
   `scripts/audit_daemon.sh status` prints.
