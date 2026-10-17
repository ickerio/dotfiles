# my arch dotfiles

personal use, i wouldn't recommend trying them

## Setup
 - `./install.sh` as your normal user (desktop PC), or `./install.sh --xps` for the XPS 15 9560
 - `gh auth login`, and paste `~/.ssh/id_rsa.pub` on GitHub
 - Run `start-hyprland` from the TTY (not `Hyprland`, it skips the crash watchdog)
 - Login to Brave Sync, Spotify and Claude Code in VS Code

[cairn](cairn/src/main.rs) draws battery, wifi, CPU, memory and the time onto the desktop in JetBrains Mono, with no dependencies. Run `cairn/glyphs.sh` after changing its fonts.

## XPS 15 9560
Before installing, in UEFI (`F2` on boot, see the [Arch wiki](https://wiki.archlinux.org/title/Dell_XPS_15_(9560))):
 - SATA Mode: RAID -> AHCI (otherwise the NVMe SSD isn't detected)
 - POST Behaviour: Fastboot -> Thorough
 - Disable Secure Boot

The desktop runs on the Intel GPU, the GTX 1050 is off by default:
 - `gpu-on` / `gpu-off` switch it (envycontrol, reboots), `gpu` shows the mode
 - `prime-run <app>` runs an app on it, `SUPER+G` opens Steam on it
 - `b` / `battery-health` show charge and health

Pascal needs the AUR `nvidia-580xx-dkms` driver, not `nvidia-open`. If boot or `lspci` hangs, add `acpi_rev_override=1` to the kernel parameters.

## Notes
 - [Wooting](https://help.wooting.io/article/147-configuring-device-access-for-wootility-under-linux-udev-rules)
 - [XPanel](https://github.com/teamfinalmouse/xpanel-linux-permissions) (`70-finalmouse.rules`, prefix must be below 73 for `uaccess`)

## Troubleshooting
 - XPS CPU stuck at 800MHz? Unplug charger and battery, hold power for 30s
 - Wi-Fi not connecting pre install? `ip link set wlan0 up` then `iwctl`
 - Hyprland not launching? `pacman -S mesa-utils mesa` then restart
