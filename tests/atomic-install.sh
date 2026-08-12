#!/bin/sh
set -eu

test_root=$(mktemp -d "${TMPDIR:-/tmp}/ccm-atomic-install.XXXXXX")
cleanup_root() {
  rm -rf "$test_root"
}
trap cleanup_root 0 HUP INT TERM

if [ -x /bin/true ]; then
  true_binary=/bin/true
else
  true_binary=/usr/bin/true
fi

atomic_install() (
  src=$1
  dst=$2
  previous="${dst}.previous"
  tmp=$(mktemp "${dst}.new.XXXXXX") || exit 1
  backup_tmp=
  cleanup_tmp() {
    [ -z "${tmp:-}" ] || rm -f "$tmp"
    [ -z "${backup_tmp:-}" ] || rm -f "$backup_tmp"
  }
  trap cleanup_tmp 0 HUP INT TERM
  install -m 755 "$src" "$tmp" || exit 1
  "$tmp" --version >/dev/null || exit 1
  if [ -e "$dst" ]; then
    backup_tmp=$(mktemp "${previous}.new.XXXXXX") || exit 1
    if cp -p "$dst" "$backup_tmp" && mv -f "$backup_tmp" "$previous"; then
      backup_tmp=
      :
    else
      rm -f "$backup_tmp"
    fi
  fi
  mv -f "$tmp" "$dst" || exit 1
  tmp=
)

destination=$test_root/codex-claude-mode
cp /bin/sleep "$destination"
"$destination" 2 &
running_pid=$!
cp "$true_binary" "$test_root/new"
atomic_install "$test_root/new" "$destination"
"$destination"
kill -0 "$running_pid"
wait "$running_pid"
cmp "$true_binary" "$destination"
cmp /bin/sleep "$destination.previous"

cp "$true_binary" "$test_root/still-old"
cp "$test_root/still-old" "$destination"
printf '#!/bin/sh\nexit 1\n' > "$test_root/invalid"
chmod 755 "$test_root/invalid"
if atomic_install "$test_root/invalid" "$destination"; then
  echo "invalid candidate unexpectedly installed" >&2
  exit 1
fi
cmp "$test_root/still-old" "$destination"
if find "$test_root" -name 'codex-claude-mode.new.*' | grep . >/dev/null; then
  echo "candidate temporary file was not cleaned up" >&2
  exit 1
fi

cp /bin/sleep "$destination"
cp "$true_binary" "$test_root/new"
atomic_install "$test_root/new" "$destination" &
installer_pid=$!
while kill -0 "$installer_pid" 2>/dev/null; do
  if ! cmp -s /bin/sleep "$destination" && ! cmp -s "$true_binary" "$destination"; then
    echo "destination contained a partial candidate" >&2
    exit 1
  fi
done
wait "$installer_pid"
cmp "$true_binary" "$destination"
