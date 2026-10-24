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

# The desktop app shares the catalog with the CLI, so it is rebuilt with it: an older app cannot show
# what a newer CLI records. It is a macOS bundle, assembled here and signed ad hoc.
if [[ "$(uname -s)" != Darwin ]]; then
    exit 0
fi
cargo build --release --locked -p ssh-manager-desktop
applications="${SSH_MANAGER_APP_DIR:-$HOME/Applications}"
config=crates/desktop/tauri.conf.json
name="$(plutil -extract productName raw "$config")"
identifier="$(plutil -extract identifier raw "$config")"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
mkdir -p "$applications"
staging="$(mktemp -d "$applications/.$name.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
bundle="$staging/$name.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources" "$staging/icon.iconset"
cp target/release/ssh-manager-desktop "$bundle/Contents/MacOS/"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" crates/desktop/icons/icon.png --out "$staging/icon.iconset/icon_${size}x${size}.png" >/dev/null
    if ((size <= 256)); then
        sips -z $((size * 2)) $((size * 2)) crates/desktop/icons/icon.png --out "$staging/icon.iconset/icon_${size}x${size}@2x.png" >/dev/null
    fi
done
iconutil -c icns "$staging/icon.iconset" -o "$bundle/Contents/Resources/icon.icns"
cat > "$bundle/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>ssh-manager-desktop</string>
  <key>CFBundleIdentifier</key><string>$identifier</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
codesign --force --sign - "$bundle"
installed="$applications/$name.app"
running=0
pgrep -qf "$installed/Contents/MacOS/" && running=1
if [[ -d "$installed" ]]; then
    mv "$installed" "$staging/previous.app"
fi
mv "$bundle" "$installed"
printf '已安装桌面端到 %s\n' "$installed"
if ((running)); then
    printf '桌面端正在运行，重新打开后使用新版本。\n'
fi
