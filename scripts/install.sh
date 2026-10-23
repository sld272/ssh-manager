#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
prefix="${1:-$HOME/.local}"
# The keychain grants access to the helper's exact binary. Build it unstripped so a toolchain that
# starts stripping release builds produces the same binary as before.
CARGO_PROFILE_RELEASE_STRIP=false cargo build --release --locked -p ssh-manager-credentials
cargo build --release --locked -p ssh-manager
mkdir -p "$prefix/bin"
for name in ssh-manager-credentials ssh-manager; do
    source_path="target/release/$name"
    target_path="$prefix/bin/$name"
    if [[ -f "$target_path" ]] && cmp -s "$source_path" "$target_path"; then
        continue
    fi
    # A different helper loses access to every saved password and passphrase until they are bound again.
    if [[ "$name" == ssh-manager-credentials && -f "$target_path" && "${SSH_MANAGER_REPLACE_CREDENTIALS:-}" != 1 ]]; then
        printf '保留已安装的凭据组件：新构建与它不同，替换后需要重新保存所有密码和口令。\n' >&2
        printf '确认要替换时，使用 SSH_MANAGER_REPLACE_CREDENTIALS=1 重新运行。\n' >&2
        continue
    fi
    temporary_path="$(mktemp "$prefix/bin/.$name.XXXXXX")"
    cp "$source_path" "$temporary_path"
    chmod 755 "$temporary_path"
    mv -f "$temporary_path" "$target_path"
done
printf '已安装到 %s/bin\n' "$prefix"
