#!/bin/sh
# Compat-audit daemon: a `zzzd` serving zzz's prod SPA from an isolated state
# dir and its own database, plus the `Secure`-stripping proxy in front of it.
# The walk-through that uses it is docs/compat.md.
#
#   scripts/audit_daemon.sh start     set up (idempotent), start zzzd + proxy
#   scripts/audit_daemon.sh restart   restart zzzd only (the proxy stays up)
#   scripts/audit_daemon.sh stop      stop both
#   scripts/audit_daemon.sh status    what is running, where things live
#
# Everything is configured by environment, with defaults — no state is read
# from the calling shell beyond these:
#
#   ZERVO_AUDIT_STATE_DIR    state dir (default ${XDG_STATE_HOME:-~/.local/state}/zervo_audit)
#   ZERVO_AUDIT_ZZZ_DIR      zzz checkout (default: the sibling ../zzz of this repo)
#   ZERVO_AUDIT_ZZZD         zzzd binary (default <zzz checkout>/target/debug/zzzd)
#   ZERVO_AUDIT_STATIC_DIR   built SPA (default <zzz checkout>/build)
#   ZERVO_AUDIT_DB           database name (default zervo_audit)
#   ZERVO_AUDIT_PORT         zzzd port (default 4480)
#   ZERVO_AUDIT_PROXY_PORT   proxy port (default 4481)
#
# The defaults stay clear of the `zzz` CLI's daemon (4460) and `cargo xtask
# dev` (4461). Only the processes this script started are ever signalled: each
# pid file records the pid and its kernel start time, and both must match.

set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

state_dir=${ZERVO_AUDIT_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/zervo_audit}
zzz_dir=${ZERVO_AUDIT_ZZZ_DIR:-$script_dir/../../zzz}
zzzd_bin=${ZERVO_AUDIT_ZZZD:-$zzz_dir/target/debug/zzzd}
static_dir=${ZERVO_AUDIT_STATIC_DIR:-$zzz_dir/build}
db_name=${ZERVO_AUDIT_DB:-zervo_audit}
port=${ZERVO_AUDIT_PORT:-4480}
proxy_port=${ZERVO_AUDIT_PROXY_PORT:-4481}

# The daemon's working directory: its app dir (`.zzz`) lands in here, not in
# whatever directory the script was called from.
home_dir=$state_dir/home
cookie_key_file=$state_dir/cookie_key
bootstrap_token_file=$state_dir/bootstrap_token
zzzd_pid_file=$state_dir/zzzd.pid
zzzd_log=$state_dir/zzzd.log
proxy_pid_file=$state_dir/proxy.pid
proxy_log=$state_dir/proxy.log
proxy_script=$script_dir/strip_secure_proxy.py

# How long `start` waits for /health, in tries of about 0.2s each. zzzd gets
# about half a minute: its first boot on a new database runs every migration,
# which is slow on a loaded machine. The proxy only has to bind a port.
zzzd_wait_tries=150
proxy_wait_tries=25

die() {
	printf 'audit_daemon: %s\n' "$*" >&2
	exit 1
}

check_port() {
	case $2 in
		'' | *[!0-9]*) die "$1 must be a port number, got '$2'" ;;
	esac
	if [ "$2" -lt 1 ] || [ "$2" -gt 65535 ]; then
		die "$1 must be in 1..=65535, got '$2'"
	fi
}

random_hex() {
	od -An -tx1 -N32 /dev/urandom | tr -d ' \n'
}

# Start time (clock ticks since boot) of pid $1 — field 22 of /proc/<pid>/stat,
# read after the last ')' so a comm with spaces or parens can't shift it.
pid_start() {
	sed 's/^.*) //' "/proc/$1/stat" 2>/dev/null | cut -d' ' -f20
}

# Writes "<pid> <start time>" for the just-started pid $2 to pid file $1.
record_pid() {
	echo "$2 $(pid_start "$2")" >"$1"
}

# Prints the pid recorded in pid file $1 when that exact process is still
# alive: the pid must have the recorded start time, so a reused pid — or a
# pid file naming some other process — is treated as gone and never signalled.
live_pid() {
	[ -f "$1" ] || return 1
	recorded_pid=
	recorded_start=
	read -r recorded_pid recorded_start <"$1" || true
	case $recorded_pid in
		'' | *[!0-9]*) return 1 ;;
	esac
	[ -n "$recorded_start" ] || return 1
	[ "$(pid_start "$recorded_pid")" = "$recorded_start" ] || return 1
	printf '%s\n' "$recorded_pid"
}

health() {
	curl -s -f -m 2 --noproxy '*' "http://127.0.0.1:$1/health" >/dev/null 2>&1
}

# Waits up to $3 tries for the just-started process in pid file $2 to answer
# /health on port $1. Fails as soon as the process is gone — including right
# after a /health answer, which then came from something else holding the port.
wait_for_health() {
	read -r started_pid _ <"$2"
	tries=0
	while [ "$tries" -lt "$3" ]; do
		if health "$1"; then
			sleep 0.2
			kill -0 "$started_pid" 2>/dev/null || return 1
			return 0
		fi
		kill -0 "$started_pid" 2>/dev/null || return 1
		tries=$((tries + 1))
		sleep 0.2
	done
	return 1
}

# stop_recorded <label> <pid file> — stops the recorded process if it is still
# the one this script started, and removes the pid file either way.
stop_recorded() {
	if pid=$(live_pid "$2"); then
		outcome=stopped
		kill "$pid" 2>/dev/null || true
		tries=0
		while kill -0 "$pid" 2>/dev/null; do
			tries=$((tries + 1))
			if [ "$tries" -gt 50 ]; then
				# escalate only while it's still the same process (pid and
				# start time), not a reused pid
				if live_pid "$2" >/dev/null; then
					kill -KILL "$pid" 2>/dev/null || true
				else
					outcome=left
				fi
				break
			fi
			sleep 0.2
		done
		if [ "$outcome" = stopped ]; then
			echo "stopped $1 (pid $pid)"
		else
			echo "$1 (pid $pid) did not stop and no longer matches the record — left alone"
		fi
	else
		echo "$1 is not running"
	fi
	rm -f "$2"
}

# Prints the number of accounts; a database zzzd hasn't migrated yet has no
# `account` table and counts as zero. Fails when psql does.
account_count() {
	exists=$(psql -X -q -A -t -d "$db_name" -c "select to_regclass('account') is not null") || return 1
	if [ "$exists" != t ]; then
		echo 0
		return 0
	fi
	psql -X -q -A -t -d "$db_name" -c 'select count(*) from account'
}

# Checks with no side effects: tools, ports, the zzz build, and that neither
# port is held by something this script didn't start.
validate() {
	for tool in curl psql createdb python3 od; do
		command -v "$tool" >/dev/null 2>&1 || die "'$tool' not found on PATH"
	done
	check_port ZERVO_AUDIT_PORT "$port"
	check_port ZERVO_AUDIT_PROXY_PORT "$proxy_port"
	[ "$port" != "$proxy_port" ] || die "the daemon and proxy ports must differ"
	[ -x "$zzzd_bin" ] || die "no zzzd at $zzzd_bin — run 'cargo build -p zzz_server' in the zzz checkout, or set ZERVO_AUDIT_ZZZD"
	[ -f "$static_dir/200.html" ] || die "no built SPA at $static_dir — run 'cargo xtask prod-setup' then 'gro build' in the zzz checkout, or set ZERVO_AUDIT_STATIC_DIR"
	[ -f "$proxy_script" ] || die "missing $proxy_script"
}

check_zzzd_port_free() {
	if ! live_pid "$zzzd_pid_file" >/dev/null && health "$port"; then
		die "port $port already answers /health but isn't this script's zzzd — set ZERVO_AUDIT_PORT"
	fi
}

check_proxy_port_free() {
	if ! live_pid "$proxy_pid_file" >/dev/null && health "$proxy_port"; then
		die "port $proxy_port already answers /health but isn't this script's proxy — set ZERVO_AUDIT_PROXY_PORT"
	fi
}

# Creates the state dir, cookie key, database, and bootstrap token as needed.
setup() {
	umask 077
	mkdir -p "$home_dir"
	chmod 700 "$state_dir"

	if [ ! -s "$cookie_key_file" ]; then
		random_hex >"$cookie_key_file"
		chmod 600 "$cookie_key_file"
		echo "generated a cookie key at $cookie_key_file"
	fi

	if ! psql -X -q -A -t -d "$db_name" -c 'select 1' >/dev/null 2>&1; then
		createdb "$db_name" || die "could not create database '$db_name' — is Postgres running?"
		echo "created database $db_name"
	fi

	# zzzd offers the bootstrap form only when a token file exists at boot, and
	# deletes the file once the first account is created.
	accounts=$(account_count) || die "could not count accounts in database '$db_name'"
	if [ "$accounts" = 0 ] && [ ! -s "$bootstrap_token_file" ]; then
		random_hex >"$bootstrap_token_file"
		chmod 600 "$bootstrap_token_file"
		echo "created a bootstrap token at $bootstrap_token_file"
	fi
}

start_zzzd() {
	if pid=$(live_pid "$zzzd_pid_file"); then
		echo "zzzd already running (pid $pid) on port $port"
		return 0
	fi
	check_zzzd_port_free
	(
		cd "$home_dir"
		# Keep the calling shell's zzz settings and provider keys out of the
		# audit daemon.
		unset ZZZ_PORT ZZZ_STATIC_DIR ZZZ_TRUSTED_PROXIES ZZZ_ENABLE_TEST_ACTIONS \
			PUBLIC_ZZZ_SCOPED_DIRS SECRET_ANTHROPIC_API_KEY SECRET_OPENAI_API_KEY \
			SECRET_GOOGLE_API_KEY
		export DATABASE_URL="postgres://localhost/$db_name"
		SECRET_FUZ_COOKIE_KEYS=$(cat "$cookie_key_file")
		export SECRET_FUZ_COOKIE_KEYS
		export FUZ_ALLOWED_ORIGINS="http://localhost:$port,http://127.0.0.1:$port,http://localhost:$proxy_port,http://127.0.0.1:$proxy_port"
		export FUZ_BOOTSTRAP_TOKEN_PATH="$bootstrap_token_file"
		export PUBLIC_ZZZ_DIR=.zzz
		exec nohup "$zzzd_bin" --port "$port" --static-dir "$static_dir" >>"$zzzd_log" 2>&1
	) &
	record_pid "$zzzd_pid_file" $!
	if ! wait_for_health "$port" "$zzzd_pid_file" "$zzzd_wait_tries"; then
		# don't leave a slow starter behind to take the port unrecorded
		stop_recorded zzzd "$zzzd_pid_file" >/dev/null
		echo "--- tail of $zzzd_log" >&2
		tail -n 20 "$zzzd_log" >&2 || true
		die "zzzd did not come up on port $port"
	fi
	echo "zzzd is up: http://localhost:$port/ (pid $(live_pid "$zzzd_pid_file"), log $zzzd_log)"
}

start_proxy() {
	if pid=$(live_pid "$proxy_pid_file"); then
		echo "proxy already running (pid $pid) on port $proxy_port"
		return 0
	fi
	check_proxy_port_free
	(
		cd "$state_dir"
		exec nohup python3 "$proxy_script" --listen "$proxy_port" --upstream "$port" >>"$proxy_log" 2>&1
	) &
	record_pid "$proxy_pid_file" $!
	if ! wait_for_health "$proxy_port" "$proxy_pid_file" "$proxy_wait_tries"; then
		stop_recorded proxy "$proxy_pid_file" >/dev/null
		echo "--- tail of $proxy_log" >&2
		tail -n 20 "$proxy_log" >&2 || true
		die "the proxy did not come up on port $proxy_port (is the port in use? set ZERVO_AUDIT_PROXY_PORT) — zzzd is still running on port $port; '$0 stop' clears it"
	fi
	echo "proxy is up: http://localhost:$proxy_port/ (pid $(live_pid "$proxy_pid_file"), Secure stripped from Set-Cookie)"
}

report_bootstrap() {
	if [ -s "$bootstrap_token_file" ]; then
		echo "no account yet — the page shows the bootstrap form; the token is:"
		echo "  $(cat "$bootstrap_token_file")"
	fi
}

status() {
	echo "state dir:  $state_dir"
	echo "database:   $db_name"
	if pid=$(live_pid "$zzzd_pid_file"); then
		if health "$port"; then answering=serving; else answering='not answering /health'; fi
		echo "zzzd:       pid $pid, http://localhost:$port/ ($answering)"
	else
		echo "zzzd:       not running"
	fi
	if pid=$(live_pid "$proxy_pid_file"); then
		echo "proxy:      pid $pid, http://localhost:$proxy_port/"
	else
		echo "proxy:      not running"
	fi
	report_bootstrap
}

case ${1:-} in
	start)
		validate
		check_zzzd_port_free
		check_proxy_port_free
		setup
		start_zzzd
		start_proxy
		report_bootstrap
		;;
	restart)
		validate
		setup
		stop_recorded zzzd "$zzzd_pid_file"
		start_zzzd
		;;
	stop)
		stop_recorded proxy "$proxy_pid_file"
		stop_recorded zzzd "$zzzd_pid_file"
		;;
	status)
		status
		;;
	*)
		echo "usage: $0 start|restart|stop|status" >&2
		exit 2
		;;
esac
