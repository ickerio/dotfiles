# dotfiles

## Layout
 - `config/*` is symlinked to `~/.config/*`, `bash/bashrc` to `~/.bashrc`
 - `machines/<pc|xps>/` holds everything machine specific (picked by `--xps`)
 - `udev/` rules are copied to `/etc/udev/rules.d`

## Setup
 - `./install.sh` (`--xps` on the laptop), safe to re-run
 - Add `~/.ssh/id_ed25519.pub` to GitHub
 - `sudo tailscale up`, with `--ssh` on the PC so it takes the laptop's SSH in place of sshd
 - VS Code `Remote-SSH: Connect to Host...` -> `desktop` opens the PC from the laptop
 - `mdesk` moshes into a tmux session on the PC (`Ctrl+B D` detaches, training keeps running)
 - `start-hyprland` from the TTY (not `Hyprland`, it skips the crash watchdog)
 - Log in to Brave Sync, Spotify, Claude Code

## Reinstall
Only `@` and `@log` get replaced, `@home` and `@pkg` are kept. From the ISO:
```
mount -o subvol=@home /dev/nvme0n1p2 /mnt && cp /mnt/harry/dotfiles/reinstall/* /root && umount /mnt
/root/prep.sh        # [btrfs partition] [EFI partition]
archinstall --config /root/config.json
```
Set hostname, recreate user `harry`, reboot, `install.sh`. Then delete the old system:
```
sudo mount -o subvolid=5 /dev/nvme0n1p2 /mnt && sudo btrfs subvolume delete -R /mnt/@old-* /mnt/@log-old-* && sudo umount /mnt
```

## cairn
[cairn](cairn/src/main.rs) draws battery, wifi, CPU, memory and clock on the desktop. Run `cairn/glyphs.sh` after changing fonts.

## XPS
UEFI (`F2`, [wiki](https://wiki.archlinux.org/title/Dell_XPS_15_(9560))): SATA RAID -> AHCI, Fastboot -> Thorough, Secure Boot off

 - `gpu-on` / `gpu-off` (reboots), `gpu` shows mode
 - `prime-run <app>`, `SUPER+G` for Steam on the 1050
 - `b` / `battery-health`
 - Boot or `lspci` hangs: `acpi_rev_override=1` kernel param
 - CPU stuck at 800MHz: unplug charger and battery, hold power 30s

## Notes
 - [Wooting](https://help.wooting.io/article/147-configuring-device-access-for-wootility-under-linux-udev-rules) / [XPanel](https://github.com/teamfinalmouse/xpanel-linux-permissions) udev rules (prefix below 73 for `uaccess`)
 - Wi-Fi on the ISO: `ip link set wlan0 up`, then `iwctl`
 - Hyprland won't launch: `pacman -S mesa mesa-utils`
