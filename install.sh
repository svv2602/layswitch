#!/usr/bin/env bash
# Builds layswitch and installs it as a systemd user service.
# One-time system setup (udev rule, input group) needs sudo.
set -euo pipefail
cd "$(dirname "$0")"

export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release
install -Dm755 target/release/layswitch "$HOME/.local/bin/layswitch"
install -Dm644 dist/layswitch.service "$HOME/.config/systemd/user/layswitch.service"
[ -f "$HOME/.config/layswitch/config.toml" ] || install -Dm644 dist/config.toml "$HOME/.config/layswitch/config.toml"

need_relogin=0
if [ ! -f /etc/udev/rules.d/70-layswitch-uinput.rules ]; then
    sudo install -m644 dist/70-layswitch-uinput.rules /etc/udev/rules.d/
    sudo udevadm control --reload-rules
    sudo udevadm trigger --name-match=uinput
fi
if ! id -nG "$USER" | grep -qw input; then
    sudo usermod -aG input "$USER"
    need_relogin=1
fi
command -v wl-paste >/dev/null || sudo apt-get install -y wl-clipboard

systemctl --user daemon-reload
if [ "$need_relogin" = 1 ]; then
    systemctl --user enable layswitch.service
    echo "Added $USER to the input group: log out and back in, the service starts with the session."
else
    systemctl --user enable --now layswitch.service
    systemctl --user restart layswitch.service
    echo "layswitch is running: journalctl --user -u layswitch -f"
fi
