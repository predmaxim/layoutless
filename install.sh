#!/usr/bin/env bash
# Build the layoutless daemon from this checkout and install it for the current user.
#
#   ./install.sh               install or update
#   ./install.sh --system      install as a system service instead: no `input` group for your user
#   ./install.sh --uninstall   remove the daemon (keeps your settings and learned words)
#   --yes                      don't ask before using sudo
#
# User files: ~/.local/bin/layoutless, ~/.config/systemd/user/layoutless.service.
# System files (sudo): /etc/udev/rules.d/60-layoutless.rules, /etc/modules-load.d/layoutless.conf,
# and your user added to the `input` group.
# --system (sudo): /usr/local/bin/layoutless, /etc/systemd/system/layoutless@.service and the same
# udev rule and module; the daemon runs as you with the `input` group for itself only.

set -euo pipefail
cd "$(dirname "$0")"

BIN="$HOME/.local/bin/layoutless"
UNIT="$HOME/.config/systemd/user/layoutless.service"
UDEV_RULE=/etc/udev/rules.d/60-layoutless.rules
MODULES_CONF=/etc/modules-load.d/layoutless.conf
SYS_BIN=/usr/local/bin/layoutless
SYS_UNIT=/etc/systemd/system/layoutless@.service

yes=false
action=do_install
for arg in "$@"; do
  case "$arg" in
    --yes | -y) yes=true ;;
    --system) action=do_install_system ;;
    --uninstall) action=do_uninstall ;;
    -h | --help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

confirm() {
  $yes && return 0
  read -r -p "$1 [y/N] " answer
  [[ $answer == [yY]* ]]
}

in_input_group() { id -nG "$USER" | tr ' ' '\n' | grep -qx input; }

build() {
  command -v hyprctl >/dev/null || die "Hyprland (hyprctl) not found"
  command -v cargo >/dev/null || die "Rust is needed to build: omarchy pkg add rustup && rustup default stable"

  say "Building the daemon"
  cargo build --release --locked --manifest-path daemon/Cargo.toml
}

install_devices() {
  sudo install -Dm644 dist/60-layoutless.rules "$UDEV_RULE"
  sudo install -Dm644 dist/modules-load.conf "$MODULES_CONF"
  sudo modprobe uinput
  sudo udevadm control --reload
  sudo udevadm trigger --subsystem-match=misc --sysname-match=uinput
}

do_install() {
  build

  say "Installing $BIN and the systemd user service"
  install -Dm755 daemon/target/release/layoutless "$BIN"
  sed "s|/usr/bin/layoutless|%h/.local/bin/layoutless|" dist/layoutless.service | install -Dm644 /dev/stdin "$UNIT"

  echo
  echo "layoutless needs to read your keyboards and type through /dev/uinput. With sudo this will:"
  echo "  - install $UDEV_RULE and $MODULES_CONF"
  echo "  - load the uinput module now"
  in_input_group || echo "  - add $USER to the 'input' group (takes effect after you log out and back in)"
  confirm "Continue?" || die "stopped; nothing was changed outside your home directory"

  install_devices

  local relogin=false
  if ! in_input_group; then
    sudo usermod -aG input "$USER"
    relogin=true
  fi

  systemctl --user daemon-reload
  systemctl --user enable layoutless >/dev/null
  # This login session only gets the input group after a re-login.
  if id -nG | tr ' ' '\n' | grep -qx input; then
    systemctl --user restart layoutless
    say "layoutless is running"
  else
    relogin=true
  fi

  $relogin && say "Log out and back in: the daemon starts by itself afterwards"
  if command -v omarchy >/dev/null; then
    say "Bar widget: omarchy plugin add https://github.com/kladaFOX/layoutless --enable"
  fi
}

do_install_system() {
  build

  echo
  echo "layoutless will run as a system service as $USER, with the 'input' group for the daemon only."
  echo "With sudo this will:"
  echo "  - install $SYS_BIN (root-owned: your programs can't swap what runs with the 'input' group)"
  echo "  - install $SYS_UNIT and enable layoutless@$USER"
  echo "  - install $UDEV_RULE and $MODULES_CONF and load the uinput module"
  confirm "Continue?" || die "stopped; nothing was changed outside your home directory"

  # A user-service install would run a second daemon next to this one.
  systemctl --user disable --now layoutless 2>/dev/null || true
  rm -f "$BIN" "$UNIT"
  systemctl --user daemon-reload

  sudo install -Dm755 daemon/target/release/layoutless "$SYS_BIN"
  sudo install -Dm644 dist/layoutless@.service "$SYS_UNIT"
  install_devices
  sudo systemctl daemon-reload
  sudo systemctl enable "layoutless@$USER" >/dev/null
  sudo systemctl restart "layoutless@$USER"
  say "layoutless is running (systemctl status layoutless@$USER)"
}

do_uninstall() {
  say "Stopping and removing the daemon"
  systemctl --user disable --now layoutless 2>/dev/null || true
  rm -f "$BIN" "$UNIT"
  systemctl --user daemon-reload

  if [[ -e $SYS_UNIT || -e $SYS_BIN ]]; then
    confirm "Remove the system service layoutless@$USER and $SYS_BIN with sudo?" && {
      sudo systemctl disable --now "layoutless@$USER" 2>/dev/null || true
      sudo rm -f "$SYS_UNIT" "$SYS_BIN"
      sudo systemctl daemon-reload
    }
  fi

  if [[ -e $UDEV_RULE || -e $MODULES_CONF ]]; then
    confirm "Remove $UDEV_RULE and $MODULES_CONF with sudo?" && sudo rm -f "$UDEV_RULE" "$MODULES_CONF"
  fi

  say "Kept: ~/.config/layoutless (settings) and ~/.local/share/layoutless (learned words)"
  say "Bar widget, if installed: omarchy plugin remove kladafox.layoutless"
  say "Leave the input group, if nothing else needs it: sudo gpasswd -d $USER input"
}

"$action"
