#!/bin/sh
# Prints the Servo source sites behind the known gaps in docs/compat.md, read
# from the checkout cargo cached for the revision `Cargo.lock` pins. Run it
# after bumping the pin to see whether an upstream gap is fixed; each section
# says what "still present" looks like. Read-only.
#
#   scripts/servo_gap_sites.sh
#
# CARGO_HOME is honoured (default ~/.cargo). A file that moved upstream is
# reported as missing rather than failing the run — find its new home and
# update this script and the doc.

set -eu

repo_dir=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
lock=$repo_dir/Cargo.lock
cargo_home=${CARGO_HOME:-$HOME/.cargo}

[ -f "$lock" ] || {
	echo "servo_gap_sites: no $lock" >&2
	exit 1
}

rev=$(sed -n '/^name = "servo"$/,/^$/s/^source = "git+.*#\([0-9a-f]\{40\}\)"$/\1/p' "$lock")
[ -n "$rev" ] || {
	echo "servo_gap_sites: no pinned servo revision in $lock" >&2
	exit 1
}
short=$(printf '%s' "$rev" | cut -c1-7)

servo=
for candidate in "$cargo_home"/git/checkouts/servo-*/"$short"*; do
	if [ -d "$candidate/components/servo" ]; then
		servo=$candidate
		break
	fi
done
[ -n "$servo" ] || {
	echo "servo_gap_sites: no cached checkout for $short under $cargo_home/git/checkouts — build first (cargo build -p zervo --locked)" >&2
	exit 1
}

echo "pinned Servo rev: $rev"
echo "checkout:         $servo"

section() {
	printf '\n== %s\n   still present when: %s\n' "$1" "$2"
}

# show <file relative to the checkout> <grep pattern>
show() {
	if [ ! -f "$servo/$1" ]; then
		echo "$1: missing (moved upstream?)"
		return 0
	fi
	grep_status=0
	matches=$(grep -n -- "$2" "$servo/$1") || grep_status=$?
	if [ "$grep_status" = 1 ]; then
		echo "$1: no match for '$2' (gone — fixed, or moved)"
		return 0
	elif [ "$grep_status" != 0 ]; then
		echo "$1: could not be read (grep exited $grep_status)"
		return 0
	fi
	printf '%s\n' "$matches" | sed "s|^|$1:|"
}

printf '\n== Rust toolchain\n   informational: this repo'"'"'s pinned rustc should be at least Cargo.toml'"'"'s rust-version,\n   ideally at or above Servo'"'"'s pinned channel\n'
show rust-toolchain.toml '^channel'
show Cargo.toml '^rust-version'
printf 'this repo'"'"'s toolchain (rust-toolchain.toml): %s\n' "$(cd "$repo_dir" && rustc --version 2>/dev/null || echo 'rustc not found')"

section 'Secure cookies on http://localhost' \
	'the cookie sites call is_secure_scheme(), which is only https / wss'
show components/net/cookie.rs 'is_secure_scheme'
show components/net/cookie_storage.rs 'is_secure_scheme'
show components/url/lib.rs 'scheme == "https" || scheme == "wss"'

section 'keypress.charCode is always 0' \
	'the platform keyboard event constructor passes a literal 0 as char_code'
show components/script/dom/event/keyboardevent.rs '/\* char_code \*/'

section 'Page text selection' \
	'selection() covers only <input> / <textarea>, and the selection-UI TODOs remain'
show components/script/dom/node/node.rs 'selection_for_layout'
show components/script/dom/selection.rs 'TODO: When implementing actual selection UI'

section 'navigator.clipboard off by default' \
	'dom_async_clipboard_enabled defaults to false'
show components/config/prefs.rs 'dom_async_clipboard_enabled'

section 'Session persistence needs a config dir' \
	'Opts::config_dir exists and defaults to None (Zervo must pass one)'
show components/config/opts.rs 'config_dir'

section 'Clipboard backend' \
	'arboard is locked with x11rb only (no wl-clipboard-rs in Cargo.lock)'
sed -n '/^name = "arboard"$/,/^$/p' "$lock" | grep -n 'x11rb\|wl-clipboard' || echo 'arboard: neither backend listed'
grep -c '^name = "wl-clipboard-rs"$' "$lock" | sed 's/^/wl-clipboard-rs packages in Cargo.lock: /'

section 'Upstream apt packages missing from zap.ts' \
	'a package is listed below (libfreetype6-dev is a deliberate rename, not a gap)'
apt_list=python/servo/platform/linux_packages/apt/apt_common.txt
if [ -f "$servo/$apt_list" ]; then
	missing=0
	while IFS= read -r pkg; do
		case $pkg in
			'' | '#'*) continue ;;
		esac
		if ! grep -q -- "'$pkg'" "$repo_dir/zap.ts"; then
			echo "not in zap.ts: $pkg"
			missing=1
		fi
	done <"$servo/$apt_list"
	[ "$missing" = 1 ] || echo 'zap.ts covers the list'
else
	echo "$apt_list: missing (moved upstream?)"
fi
