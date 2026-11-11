# Zervo

> native Zzz window host using Servo (experimental)

**Binary**: `zervo` (`crates/zervo`) | **Target**: Linux on Wayland |
**Status**: an experiment, not ready for use

For coding conventions, see the [fuz-stack skill](https://docs.fuz.dev/skills/fuz-stack). For what Zervo is and what works,
see `README.md`; for the known gaps and the Servo pin, `docs/compat.md`. This
file is the orientation for working on the repo.

## What Zervo is

A separate process that opens a winit window, embeds a Servo `WebView`, and
loads Zzz's web app from a running Zzz daemon over loopback. It never embeds
the daemon. The zzz CLI can launch it through its generic opener (`opener` in
`~/.zzz/config.json`, or `ZZZ_OPENER`), which passes the URL as the final
argument; Zzz never names Zervo. Servo is the only engine: there's no engine
seam in the code, and another engine would be another binary.

Zervo is its own Cargo workspace, not a member of Zzz's, so Servo's dependency
graph, lockfile, and feature unification stay out of the daemon.

## Layout

```
crates/zervo/src/
├── main.rs         # URL resolution, window + engine setup, event dispatch, paint
├── render_loop.rs  # the waker Servo calls, and the loop's activity counts
├── input.rs        # pointer, wheel, touch, cursor, theme: winit → Servo
└── keyutils.rs     # keyboard: winit → Servo
docs/compat.md      # known gaps, rebuilding, bumping the Servo pin, the walk-through
scripts/            # the compat audit harness
zap.ts              # Servo's build toolchain (Debian packages)
zap.lock            # pins the fgro build that evaluates zap.ts (`zap lock` writes it)
package.json        # marks zap.ts's project for Zap; nothing to install
```

The module docs (`//!`) are the detail; update them with the code.

## Invariants

Each is explained where it lives; know them before touching `main.rs`.

- **Drop order.** `Servo`'s `Drop` touches the compositor's GL context, so the
  `WebView` and `Servo` drop before the rendering context and the window:
  explicitly on `CloseRequested`, and by `App`'s field order as the backstop.
  Wrong order segfaults on close. Never keep a `WebView` handle a delegate
  callback receives; each one keeps `Servo` alive.
- **Paint sequence.** `make_current()` → `webview.paint()` →
  `window.pre_present_notify()` → `present()`, only on `RedrawRequested`.
  `pre_present_notify` is what stops a hidden window from blocking the thread
  on its second buffer swap (`App::paint`).
- **Resize through the `WebView`.** `webview.resize` resizes the rendering
  context and tells the page; resizing the rendering context directly leaves
  the page at its old size.
- **HiDPI scale is a `WebView` property**, set after the build and on
  `ScaleFactorChanged` (`set_hidpi_scale_factor`, which avoids depending on
  `euclid`).
- **The loop waits.** `ControlFlow::Wait`; Servo wakes it through the `Waker`
  in `render_loop.rs`, and Servo is spun on each wake-up and after each window
  event.
- **No catch-all in `window_event`.** Every winit `WindowEvent` variant is
  forwarded, handled, or listed as ignored with its reason, so a winit bump
  that adds one fails to compile.
- **Every key goes to the page.** No host shortcuts, so a terminal in the page
  sees Ctrl+C.
- **`raw-window-handle` unifies with Servo's.** It's the seam
  (`WindowRenderingContext::new` takes raw handles), so winit's version is
  independent of Servo's.

## Servo

`servo` is a git dependency on `main` with no semver release; `Cargo.lock`
pins the revision, so build with `--locked`. Its embedding API moves: on each
pin bump, reconcile `crates/zervo/src/` against the new checkout under
`~/.cargo/git/checkouts/servo-*/<rev>/` (`components/servo/` and
`ports/servoshell/`, which `input.rs` and `keyutils.rs` mirror), reading the
symbols rather than guessing. Never run cargo inside that checkout. The
procedure is `docs/compat.md` § Bumping the Servo pin.

`docs/compat.md` is the living gap list. Keep it current when a gap is fixed
or found, with results tied to the Servo revision they were observed at.

## Commands

```bash
cargo fmt --check                                 # rustfmt defaults, no config
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked   # pedantic + nursery, no warnings
cargo test --workspace --locked
cargo build -p zervo --locked
./target/debug/zervo [url]       # run the binary directly; `cargo run` relinks slowly
scripts/audit_daemon.sh start    # an isolated zzz daemon + the login workaround proxy
scripts/servo_gap_sites.sh       # the Servo source sites behind the upstream gaps
```

The first build compiles all of Servo, so CI (`.github/workflows/check.yml`)
runs only what needs no Servo build: rustfmt, and a syntax check of the
scripts. Run clippy and the tests locally before pushing.

Against a live daemon, always use Zzz's production build served by its daemon,
never the Vite dev server (its cross-origin module graph fails Servo's CORS
check).

`RUST_LOG=zervo=debug` logs resize, scale, theme, the first painted frame,
and a `loop activity` line about once a second while the loop is busy.
`RUST_LOG=zervo=trace` adds every forwarded input event, typed passwords
included.

## Testing

Unit tests cover the pure functions: URL resolution in `main.rs`, the input
and keyboard mapping, and the loop counts. Everything that needs a window is
checked by hand with the walk-through in `docs/compat.md`, which opens a real
window.
