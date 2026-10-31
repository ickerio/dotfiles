# dotfiles

## Commands
 - `./install.sh [--xps]`
 - `sudo tailscale up [--ssh]`
 - `start-hyprland`
 - `mdesk [session]`, `Ctrl+B D`
 - `claude`, `/remote-control`
 - `cairn/glyphs.sh`
 - PC: `aux`, `speakers`
 - XPS: `gpu`, `gpu-on`, `gpu-off`, `prime-run`, `battery-health`

## Keys
 - `SUPER` + `Q` close, `Backspace` exit, `V` float, `F` fullscreen
 - `SUPER` + `D` foot, `W` brave, `C` code, `S` spotify, `M` prism, `G` steam (XPS)
 - `SUPER` + `SHIFT` + `S` region screenshot, `Print` full
 - `SUPER` + `1-0` workspace, `SHIFT` to move, drag/`SHIFT` drag to move/resize

## Reinstall
```
mount -o subvol=@home /dev/nvme0n1p2 /mnt && cp /mnt/harry/dotfiles/reinstall/* /root && umount /mnt
/root/prep.sh        # [btrfs partition] [EFI partition]
archinstall --config /root/config.json
```
```
sudo mount -o subvolid=5 /dev/nvme0n1p2 /mnt && sudo btrfs subvolume delete -R /mnt/@old-* /mnt/@log-old-* && sudo umount /mnt
```

## Fixes
 - XPS UEFI: AHCI, Fastboot Thorough, Secure Boot off
 - XPS hangs: `acpi_rev_override=1`
 - XPS 800MHz: unplug charger + battery, hold power 30s
 - ISO Wi-Fi: `ip link set wlan0 up`, `iwctl`
 - No Hyprland: `pacman -S mesa mesa-utils`
 - udev prefix < 73
