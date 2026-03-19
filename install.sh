#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
PATH_LINE="export PATH=\"${BIN_DIR}:\$PATH\""

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required but was not found on PATH" >&2
  echo "install Rust from https://rustup.rs and try again" >&2
  exit 1
fi

if ! command -v jj >/dev/null 2>&1; then
  echo "warning: jj was not found on PATH" >&2
  echo "lazyjj installs successfully without it, but the app requires jj at runtime" >&2
fi

add_path_to_profile() {
  local profile="$1"
  local marker="# lazyjj installer: cargo bin path"

  if [[ ! -f "${profile}" ]]; then
    touch "${profile}"
  fi

  if grep -Fq "${PATH_LINE}" "${profile}"; then
    return 0
  fi

  {
    printf "\n%s\n" "${marker}"
    printf "%s\n" "${PATH_LINE}"
  } >>"${profile}"
}

read_package_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' "${SCRIPT_DIR}/Cargo.toml" | head -n 1
}

read_installed_version() {
  cargo install --list 2>/dev/null | sed -n 's/^lazyjj v\([^ (]*\).*/\1/p' | head -n 1
}

profile_targets=()
case "$(basename "${SHELL:-}")" in
  bash)
    profile_targets+=("$HOME/.bashrc" "$HOME/.bash_profile")
    ;;
  zsh)
    profile_targets+=("$HOME/.zshrc" "$HOME/.zprofile")
    ;;
  fish)
    :
    ;;
  *)
    :
    ;;
esac
profile_targets+=("$HOME/.profile")

PACKAGE_VERSION="$(read_package_version)"
INSTALLED_VERSION="$(read_installed_version || true)"
INSTALL_STATUS="installed"

if [[ -z "${PACKAGE_VERSION}" ]]; then
  echo "error: failed to read lazyjj version from Cargo.toml" >&2
  exit 1
fi

if [[ ":$PATH:" != *":${BIN_DIR}:"* ]]; then
  export PATH="${BIN_DIR}:$PATH"
fi

if [[ -n "${INSTALLED_VERSION}" && "${INSTALLED_VERSION}" == "${PACKAGE_VERSION}" ]]; then
  echo "lazyjj v${PACKAGE_VERSION} is already installed globally; skipping reinstall"
  INSTALL_STATUS="already installed"
else
  echo "Installing lazyjj v${PACKAGE_VERSION} from ${SCRIPT_DIR}"
  cargo install --path "${SCRIPT_DIR}" --locked
fi

for profile in "${profile_targets[@]}"; do
  add_path_to_profile "${profile}"
done

deduped_profiles=()
for profile in "${profile_targets[@]}"; do
  skip=false
  for existing in "${deduped_profiles[@]}"; do
    if [[ "${existing}" == "${profile}" ]]; then
      skip=true
      break
    fi
  done
  if [[ "${skip}" == false ]]; then
    deduped_profiles+=("${profile}")
  fi
done

cat <<EOF

lazyjj ${INSTALL_STATUS} successfully.

Binary location:
  ${BIN_DIR}/lazyjj

Updated shell startup files:
$(printf '  %s\n' "${deduped_profiles[@]}")

Run lazyjj from inside a jj repository:
  lazyjj
EOF
