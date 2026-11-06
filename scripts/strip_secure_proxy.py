#!/usr/bin/env python3
"""Compat-audit workaround: a loopback TCP pass-through that drops `Secure`
from `Set-Cookie` response headers.

Servo ignores a `Secure` cookie set over `http://localhost` (see the cookie
entry in docs/compat.md), so zzz's session cookie never sticks and nothing
behind the login can be audited. Pointing Zervo at this proxy instead of
at `zzzd` lets the rest of the walk-through run until that gap is fixed.

It exists for that one gap and nothing else: it is not a general proxy, it
weakens the cookie it rewrites, and it should never sit in front of a daemon
that holds anything real. Delete it once Servo accepts `Secure` cookies on
loopback.

Client-to-server bytes pass through untouched. Server-to-client bytes are
scanned per received chunk with no HTTP parsing: any line that starts with
`Set-Cookie:` loses its `Secure` attribute, wherever it is — a response body
or WebSocket frame containing such a line would be altered too (not something
zzz serves; JSON escapes newlines). A `Set-Cookie` line split across two reads
keeps its `Secure` — fine for the small responses that set cookies.

Usage: strip_secure_proxy.py [--listen PORT] [--upstream PORT]
"""

import argparse
import asyncio
import re

HOST = "127.0.0.1"
DEFAULT_LISTEN_PORT = 4481
DEFAULT_UPSTREAM_PORT = 4480

SET_COOKIE_LINE = re.compile(rb"^set-cookie:[^\r\n]*", re.IGNORECASE | re.MULTILINE)
SECURE_ATTRIBUTE = re.compile(rb";[ \t]*secure(?=[ \t]*(?:;|$))", re.IGNORECASE)


def strip_secure(data: bytes) -> bytes:
    """Removes the `Secure` attribute from each `Set-Cookie` line in `data`."""
    if b"set-cookie" not in data.lower():
        return data
    return SET_COOKIE_LINE.sub(lambda m: SECURE_ATTRIBUTE.sub(b"", m.group(0)), data)


async def pipe(
    reader: asyncio.StreamReader, writer: asyncio.StreamWriter, rewrite: bool
) -> None:
    try:
        while data := await reader.read(65536):
            writer.write(strip_secure(data) if rewrite else data)
            await writer.drain()
    except (ConnectionError, asyncio.CancelledError):
        pass
    finally:
        writer.close()


async def serve(listen_port: int, upstream_port: int) -> None:
    async def handle(
        client_reader: asyncio.StreamReader, client_writer: asyncio.StreamWriter
    ) -> None:
        try:
            upstream_reader, upstream_writer = await asyncio.open_connection(
                HOST, upstream_port
            )
        except OSError:
            client_writer.close()
            return
        await asyncio.gather(
            pipe(client_reader, upstream_writer, rewrite=False),
            pipe(upstream_reader, client_writer, rewrite=True),
        )

    server = await asyncio.start_server(handle, HOST, listen_port)
    print(
        f"strip_secure_proxy: http://{HOST}:{listen_port} -> http://{HOST}:{upstream_port}",
        flush=True,
    )
    async with server:
        await server.serve_forever()


def port(value: str) -> int:
    number = int(value)
    if not 1 <= number <= 65535:
        raise argparse.ArgumentTypeError(f"not a port in 1..=65535: {value}")
    return number


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Loopback pass-through that drops `Secure` from Set-Cookie "
        "(compat-audit workaround, see docs/compat.md)."
    )
    parser.add_argument(
        "--listen",
        type=port,
        default=DEFAULT_LISTEN_PORT,
        help=f"port to listen on (default {DEFAULT_LISTEN_PORT})",
    )
    parser.add_argument(
        "--upstream",
        type=port,
        default=DEFAULT_UPSTREAM_PORT,
        help=f"zzzd port to forward to (default {DEFAULT_UPSTREAM_PORT})",
    )
    args = parser.parse_args()
    try:
        asyncio.run(serve(args.listen, args.upstream))
    except KeyboardInterrupt:
        pass
    except OSError as error:
        raise SystemExit(f"strip_secure_proxy: {error}") from None


if __name__ == "__main__":
    main()
