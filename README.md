# my arch dotfiles

intended for personal use i wouldn't reccomend trying them

## Setup
 - Execute `./install.sh` as your normal user (desktop PC), or `./install.sh --xps` for the Dell XPS 15 9560
 - Add git config `git config --global user.name "ickerio"` and `git config --global user.email "private@email.com"`
 - Paste `/home/ickerio/.ssh/id_rsa.pub` on GitHub
 - Run `start-hyprland` from the TTY (not `Hyprland`, it skips the crash watchdog)
 - Setup Brave Sync, login to Spotify, login to Claude Code in VS Code
 - `gh auth login`

Hyprland config is Lua: shared `hypr/hyprland.lua`, plus `hypr/machines/<machine>.lua` which install.sh symlinks to `hypr/machine.lua`.

Packages come from the official repos where possible, the AUR (via `yay-bin`) only for what isn't there. VS Code is the official `code` package (open source build, extensions from Open VSX).


## XPS 15 9560
Before installing, in UEFI settings (`F2` on boot) per the [Arch wiki](https://wiki.archlinux.org/title/Dell_XPS_15_(9560)):
 - SATA Mode: RAID -> AHCI (otherwise the NVMe SSD isn't detected)
 - POST Behaviour: Fastboot -> Thorough
 - Disable Secure Boot

Hyprland runs on the Intel GPU (all outputs are wired to it). The GTX 1050 is off by default to save battery:
 - `gpu-on` turns it on (envycontrol hybrid mode) and reboots, then run apps on it with `prime-run <app>`. `SUPER+G` opens Steam on it.
 - `gpu-off` turns it off (envycontrol integrated mode) and reboots
 - `gpu` shows the current mode

Pascal needs the legacy `nvidia-580xx-dkms` driver from the AUR, not `nvidia-open`. If it hangs on boot or `lspci`, add `acpi_rev_override=1` to the kernel parameters.

## Notes
 - [Wooting](https://help.wooting.io/article/147-configuring-device-access-for-wootility-under-linux-udev-rules)
 - [XPanel](https://github.com/teamfinalmouse/xpanel-linux-permissions) (`70-finalmouse.rules`, the prefix must be below 73 for `uaccess` to work)

# Troubleshooting
 - CPU stuck at 800MHz on the XPS? Unplug charger and battery, hold power for 30s
 - Wi-Fi not connecting pre install? `ip link set wlan0 up` then `iwctl`
 - Hyprland not launching? `pacman -S mesa-utils mesa` then restart
