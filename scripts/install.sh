#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
prefix="${1:-$HOME/.local}"
cargo build --release --locked -p ssh-manager-credentials
cargo build --release --locked -p ssh-manager
mkdir -p "$prefix/bin"
for name in ssh-manager-credentials ssh-manager; do
    source_path="target/release/$name"
    target_path="$prefix/bin/$name"
    if [[ -f "$target_path" ]] && cmp -s "$source_path" "$target_path"; then
        continue
    fi
    temporary_path="$(mktemp "$prefix/bin/.$name.XXXXXX")"
    cp "$source_path" "$temporary_path"
    chmod 755 "$temporary_path"
    mv -f "$temporary_path" "$target_path"
done
printf '已安装到 %s/bin\n' "$prefix"
