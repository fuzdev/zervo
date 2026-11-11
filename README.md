# Zervo

> native [Zzz](https://github.com/fuzdev/zzz) window host using [Servo](https://servo.org) (experimental)

**Zervo is an experiment.** It isn't ready for use: Zzz works in a browser,
and Zervo renders it with known gaps. The question it answers is whether Servo
can host Zzz well enough to replace the browser tab.

Zervo is part of the Fuz stack
([fuz.dev](https://www.fuz.dev/), [@fuzdev](https://github.com/fuzdev)).

AI disclosure: this codebase is LLM-generated with uneven human review.

## What it is

Zzz runs as a local daemon serving a web app over loopback. Zervo is a
separate program that opens a Wayland window with
[winit](https://github.com/rust-windowing/winit), embeds Servo in it, and
loads that app. It forwards keyboard, mouse, wheel, and touch input to the
page, and sleeps until a window event or the engine wakes it.

- **A separate process.** Zervo never embeds the daemon, so a crash in the
  engine can't take down the daemon and the terminals it holds.
- **Servo only.** There's no seam for another engine.
- **Linux on Wayland** is the only target.

## Status

Against Zzz's production build, Zervo hydrates the app, holds its WebSocket,
and runs a terminal. It can't log in without a workaround, because Servo
drops `Secure` cookies on `http://localhost`. The terminal drops spaces and
some other characters, page text can't be selected, and context menus,
`<select>` popups, dialogs, and IME aren't wired.

[docs/compat.md](docs/compat.md) lists every known gap with its cause and
owner, and how to recheck it.

## Building

Servo compiles from source and needs a large native toolchain (Debian
packages). [Zap](https://github.com/fuzdev/zap) installs it from `zap.ts`:

```sh
zap apply            # preview what would be installed
zap apply --wetrun   # install it
```

Then build:

```sh
cargo build -p zervo --locked
```

rustup installs the Rust toolchain pinned in `rust-toolchain.toml`. The first
build compiles all of Servo and takes a long time; later builds are
incremental.

## Running

```sh
./target/debug/zervo                           # a built-in stub page
./target/debug/zervo http://localhost:4460/    # a running zzz daemon
ZERVO_URL=http://localhost:4460/ ./target/debug/zervo
```

The URL comes from the final argument, then `ZERVO_URL`, then the stub page.
A bad URL exits with status 2 before any window opens. The zzz CLI can launch
Zervo instead of the browser: set its `opener` (in `~/.zzz/config.json`, or
`ZZZ_OPENER`) to the path of the `zervo` binary, and zzz passes the URL as the
final argument.

`RUST_LOG=zervo=debug` logs the event loop's activity on stderr.
`RUST_LOG=zervo=trace` also logs every forwarded input event, including typed
passwords.

## Layout

```
crates/zervo/   the binary: winit window, Servo embedding, input mapping
docs/compat.md  known gaps, the Servo pin-bump procedure, a walk-through
scripts/        the compat audit harness
zap.ts          Servo's build toolchain
zap.lock        pins the fgro build that evaluates zap.ts
package.json    marks zap.ts's project for Zap (nothing to install)
```

## Contributing

[fuz.dev/contributing](https://www.fuz.dev/contributing)

## Security

Report vulnerabilities through the [fuzdev security policy](https://github.com/fuzdev/.github/blob/main/SECURITY.md).

## License [🐦](https://wikipedia.org/wiki/Free_and_open-source_software)

[0BSD](LICENSE)
