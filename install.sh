#!/bin/sh
set -eu

repo="https://github.com/ColinVaughn/Synaptic"
install_dir="${SYNAPTIC_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s):$(uname -m)" in
  Linux:x86_64|Linux:amd64) target="x86_64-unknown-linux-gnu" ;;
  Darwin:x86_64|Darwin:amd64) target="x86_64-apple-darwin" ;;
  Darwin:arm64|Darwin:aarch64) target="aarch64-apple-darwin" ;;
  *) echo "Synaptic has no prebuilt release for $(uname -s) $(uname -m)." >&2; exit 1 ;;
esac

archive="synaptic-$target.tar.gz"
base="$repo/releases/latest/download"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

if command -v curl >/dev/null 2>&1; then
  fetch() { curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q "$1" -O "$2"; }
else
  echo "Install curl or wget, then try again." >&2
  exit 1
fi

echo "Downloading the latest Synaptic release..."
fetch "$base/$archive" "$tmp/$archive"
fetch "$base/$archive.sha256" "$tmp/$archive.sha256"

expected="$(awk 'NR == 1 { print $1 }' "$tmp/$archive.sha256" | tr '[:upper:]' '[:lower:]')"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp/$archive" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$tmp/$archive" | awk '{ print $1 }')"
else
  echo "A SHA-256 tool (sha256sum or shasum) is required." >&2
  exit 1
fi
[ "${#expected}" -eq 64 ] && [ "$actual" = "$expected" ] || {
  echo "Checksum verification failed; nothing was installed." >&2
  exit 1
}

tar -xzf "$tmp/$archive" -C "$tmp"
bundle="$tmp/synaptic-$target"
mkdir -p "$install_dir"
for name in synaptic syn synaptic-ui; do
  [ -f "$bundle/$name" ] || continue
  staged="$install_dir/.$name.install.$$"
  cp "$bundle/$name" "$staged"
  chmod 755 "$staged"
  mv -f "$staged" "$install_dir/$name"
done

if [ "$(uname -s)" = Darwin ] && [ -d "$bundle/Synaptic.app" ]; then
  app_dir="${SYNAPTIC_APP_DIR:-$HOME/Applications}"
  mkdir -p "$app_dir"
  cp -R "$bundle/Synaptic.app" "$app_dir/"
  ln -sf "$app_dir/Synaptic.app/Contents/MacOS/synaptic-ui" "$install_dir/synaptic-ui"
fi

case ":$PATH:" in
  *":$install_dir:"*) ;;
  *)
    if [ "${SYNAPTIC_NO_PATH_UPDATE:-}" = 1 ]; then
      echo "$install_dir is not on PATH."
    else
      profile="$HOME/.profile"
      case "${SHELL:-}" in
        */zsh) profile="$HOME/.zprofile" ;;
        */bash) profile="$HOME/.bashrc" ;;
      esac
      path_line='export PATH="$HOME/.local/bin:$PATH"'
      if [ "$install_dir" = "$HOME/.local/bin" ] && ! grep -F "$path_line" "$profile" >/dev/null 2>&1; then
        printf '\n%s\n' "$path_line" >> "$profile"
        echo "PATH updated in $profile; open a new terminal before running Synaptic."
      elif [ "$install_dir" != "$HOME/.local/bin" ]; then
        echo "$install_dir is not on PATH; add it before running Synaptic by name."
      else
        echo "PATH is configured in $profile; open a new terminal before running Synaptic."
      fi
    fi
    ;;
esac

"$install_dir/synaptic" --version
echo "Installed in $install_dir"
echo "From a repository, run: synaptic extract . && synaptic install <assistant>"
