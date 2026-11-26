#!/bin/sh
# Standard input carries test files; standard output carries retained evidence.
set -eu
fail() {
    printf '%s\n' "$1" >&2
    exit 2
}

test "$(id -u)" -ne 0 || fail "Run this on a prepared runner as an ordinary user."
base=${RUSTWARD_NSPAWN_BASE_IMAGE:?Set the path to the prepared Debian 13 base image}
image=${RUSTWARD_NSPAWN_IMAGE_SLOT:-/var/lib/machines/rwtest.raw}
case "$image" in
    /var/lib/machines/*.raw) ;;
    *) fail "Use a trusted image slot in /var/lib/machines." ;;
esac
test "$(dirname "$image")" = /var/lib/machines ||
    fail "Use a slot directly in /var/lib/machines."
test -f "$base" && test -r "$base" && test ! -w "$base" ||
    fail "The base must be a readable image that the runner cannot write."
test -f "$image" && test ! -L "$image" && test ! "$base" -ef "$image" ||
    fail "Use a separate, existing regular file for the image slot."
test -w "$image" && test -O "$image" ||
    fail "The runner must own and be able to write the image slot."
test -S /run/systemd/io.systemd.NamespaceResource && test -S /run/systemd/io.systemd.MountFileSystem ||
    fail "Start the host systemd namespace and mount sockets."
test "$(stat -fc %T /sys/fs/cgroup)" = cgroup2fs || fail "The runner requires cgroup v2."
test -n "${XDG_RUNTIME_DIR:?Start the dedicated runner user manager}"

exec 9>"$XDG_RUNTIME_DIR/rustward-nspawn.lock"
flock -n 9 || { echo "The Rustward test image is already in use." >&2; exit 2; }
cp --reflink=auto --sparse=always -- "$base" "$image"

container() {
    deadline=$1 console=$2
    shift 2
    systemd-run --user --wait --pipe --collect --property=Delegate=yes \
        /usr/bin/timeout --kill-after=15s "$deadline" /usr/bin/systemd-nspawn \
        --image="$image" --machine=rwtest --settings=no --register=no --keep-unit \
        --private-network --console="$console" "$@"
}

container 120s pipe /bin/sh -ec '
    mkdir /opt/rustward-test
    tar --no-same-owner -xf - -C /opt/rustward-test
    cat > /etc/systemd/system/rustward-acceptance.service <<EOF_UNIT
[Unit]
Description=Run Rustward local acceptance
After=systemd-logind.service

[Service]
Type=oneshot
ExecStart=/usr/bin/python3 -B /opt/rustward-test/acceptance/nspawn-guest.py
ExecStopPost=/usr/bin/systemctl --no-block poweroff
StandardOutput=journal+console
StandardError=journal+console
TimeoutStartSec=1200

[Install]
WantedBy=multi-user.target
EOF_UNIT
    systemctl --root=/ enable rustward-acceptance.service
' >&2

status=0
container 1250s read-only --boot >&2 || status=$?

container 120s pipe /bin/sh -ec '
    cd /opt/rustward-test
    set --
    for path in result.json fixture/fixture.json fixture/evidence; do
        if test -e "$path"; then set -- "$@" "$path"; fi
    done
    tar -cf - "$@"
'
exit "$status"
