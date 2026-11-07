#!/bin/sh
# certbot's deploy hook for the Project Avalanche master (install as
# /etc/letsencrypt/renewal-hooks/deploy/avalanche-master.sh, mode 0755, owned by root). certbot runs it as root after
# each successful issue or renewal with RENEWED_LINEAGE set to the certificate's live folder. It copies the chain and
# the key into the master's TLS folder, owned by root and readable by the master's group only (the key 0640, the chain
# 0644; the master reads them and can change neither), each written beside its place and renamed over it, then has the
# master read them again (SIGHUP: no restart, the list of servers stays). Run it by hand once after the first
# `certbot certonly`, with RENEWED_LINEAGE set.
#
# The TLS folder and every folder above it must be the hook's user's (root's) or root's, and writable by nobody else:
# the hook writes there as root, and a folder the master's user could write (it serves the Internet) would let it
# put a link where the hook writes or swap the folder for one, between the hook's check and its write (merge review of
# round net4b, 2026-11-07). The hook refuses any other folder and copies nothing.
#
# Settings (environment, or edit here):
#   AVALANCHE_MASTER_DOMAIN  the master's name; another lineage's renewal is ignored (empty: every lineage)
#   AVALANCHE_MASTER_TLS     where master.toml's [tls] points, an absolute path (default /etc/avalanche-master/tls)
#   AVALANCHE_MASTER_GROUP   the group the master runs as, which may read the files (default avalanche-master; with
#                            Docker Compose the image's group id, 10002)
#   AVALANCHE_MASTER_RELOAD  the command that sends SIGHUP (default: systemctl reload avalanche-master; with Docker
#                            Compose: docker compose -f /path/to/compose.yaml kill -s HUP avalanche-master)
set -eu

domain="${AVALANCHE_MASTER_DOMAIN:-}"
dest="${AVALANCHE_MASTER_TLS:-/etc/avalanche-master/tls}"
group="${AVALANCHE_MASTER_GROUP:-avalanche-master}"
reload="${AVALANCHE_MASTER_RELOAD:-systemctl reload avalanche-master}"

fail() {
    echo "avalanche-master hook: $1: nothing copied" >&2
    exit 1
}

lineage="${RENEWED_LINEAGE:?certbot sets RENEWED_LINEAGE (the certificate live folder)}"
if [ -n "$domain" ] && [ "$(basename "$lineage")" != "$domain" ]; then
    exit 0
fi
for f in fullchain.pem privkey.pem; do
    if [ ! -s "$lineage/$f" ]; then
        fail "$lineage/$f is missing or empty"
    fi
done

# the TLS folder and each folder above it: a real folder (no link), the hook's user's or root's, no group or other
# write bit
me="$(id -u)"
case "$dest" in
    /*) ;;
    *) fail "$dest is not an absolute path" ;;
esac
# a plain path: a trailing slash would make the link test follow the link, a `.` or `..` part would walk other folders
# than the one written
case "$dest" in
    / | */ | *//* | */./* | */../* | */. | */..) fail "$dest is not a plain path (no trailing slash, no . or .. part)" ;;
esac
d="$dest"
while :; do
    if [ -L "$d" ] || [ ! -d "$d" ]; then
        fail "$d is not a folder (or is a link)"
    fi
    owner="$(stat -c %u "$d")"
    if [ "$owner" != "$me" ] && [ "$owner" != 0 ]; then
        fail "$d belongs to user $owner, not to root or the hook's user"
    fi
    if [ -n "$(find "$d" -maxdepth 0 -perm /022)" ]; then
        fail "$d is writable by its group or by others"
    fi
    [ "$d" = / ] && break
    d="$(dirname "$d")"
done

# one file: written beside its place (same file system), owner, group and mode set before it is visible, renamed
# over it (nobody else can write the folder, so nothing can be put in the way)
place() {
    tmp="$dest/.$2.new"
    rm -f "$tmp"
    install -o "$me" -g "$group" -m "$3" "$lineage/$1" "$tmp"
    sync "$tmp" 2>/dev/null || sync
    mv -f "$tmp" "$dest/$2"
}
# (the master reads them only when asked, after both; a pair that does not match is refused there and the one in use
# stays)
place privkey.pem privkey.pem 0640
place fullchain.pem fullchain.pem 0644
sync "$dest" 2>/dev/null || sync

# shellcheck disable=SC2086 # the command is words on purpose
$reload
echo "avalanche-master hook: copied $lineage into $dest and asked the master to read them again"
