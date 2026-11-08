# dotfiles

## Commands
 - `./install.sh [--xps]`
 - `sudo tailscale up [--ssh]`
 - `start-hyprland`
 - `claude`, `/remote-control`
 - `lookout/glyphs.sh`, `cargo run --release --features shot -- shot out.png --pick` to see the bar without restarting it
 - Brave in the bar's colours: `brave://extensions`, Developer mode, Load unpacked `~/dotfiles/brave/theme`
 - Agents: `hc` (`hc ls`, `hc attach`), XPS once: `hc machine add desktop --name desktop`
 - PC: `aux`, `speakers`
 - XPS: `gpu`, `gpu-on`, `gpu-off`, `prime-run`, `battery-health`

## Keys
 - `SUPER` + `Q` close, `Backspace` exit, `V` float, `F` fullscreen, `E` flip split
 - `SUPER` + `D` foot, `W` brave, `C` code, `S` spotify, `M` prism, `G` steam (XPS)
 - `SUPER` + `SHIFT` + `S` region screenshot, `Print` full
 - `SUPER` + `1-0` workspace, `SHIFT` to move, drag/`SHIFT` drag to move/resize
 - Agents: click a tab in the bar to go to it, `SUPER` + `A` or click the bar for every agent in detail and a search (type to narrow, Enter, Esc or `SUPER` to close), `SHIFT` + `A` new, `N` next that wants you, `SHIFT` + `Q` end it.
   `SUPER` + `Q` only closes the window, `Ctrl+\` detaches. After a reboot agents come back as shells, pick
   marks them `resume`

## Reinstall
```
mount -o subvol=@home /dev/nvme0n1p2 /mnt && cp /mnt/harry/dotfiles/reinstall/* /root && umount /mnt
/root/prep.sh        # [btrfs partition] [EFI partition]
archinstall --config /root/config.json   # set Hostname: desktop or laptop
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
