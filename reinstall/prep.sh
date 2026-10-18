#!/bin/bash
# Run from the Arch ISO as root. Replaces the system subvolume (@) and @log
# with fresh empty ones (old ones renamed, not deleted), keeps @home and
# @pkg untouched, then mounts everything at /mnt ready for:
#   archinstall --config config.json
#
# Usage: prep.sh [btrfs partition] [EFI partition]
#   defaults to /dev/nvme0n1p2 and /dev/nvme0n1p1
set -euo pipefail

ROOT_PART=${1:-/dev/nvme0n1p2}
ESP=${2:-/dev/nvme0n1p1}
OPTS=rw,relatime,ssd,discard=async,space_cache=v2

# Refuse to run if device names shifted (e.g. NVMe drives enumerated in a different order)
if [ "$(blkid -s TYPE -o value "$ROOT_PART")" != "btrfs" ]; then
    echo "ERROR: $ROOT_PART is not btrfs. Check lsblk -f." >&2
    exit 1
fi
if [ "$(blkid -s TYPE -o value "$ESP")" != "vfat" ]; then
    echo "ERROR: $ESP is not a FAT partition. Check lsblk -f." >&2
    exit 1
fi

mkdir -p /mnt/top
mount -o subvolid=5 "$ROOT_PART" /mnt/top

[ -d /mnt/top/@home ] || { echo "ERROR: @home not found, aborting" >&2; umount /mnt/top; exit 1; }

lsblk -f "$ROOT_PART" "$ESP"
echo
echo "Subvolumes on $ROOT_PART:"
btrfs subvolume list /mnt/top
echo
echo "@ and @log will be replaced (old ones kept), @home and @pkg kept, $ESP FORMATTED."
read -rp "Continue? [y/N] " answer
[ "$answer" = y ] || { umount /mnt/top; exit 1; }

# Rename rather than delete, so the old system is still recoverable
TS=$(date +%Y%m%d-%H%M%S)
mv /mnt/top/@ "/mnt/top/@old-$TS"
btrfs subvolume create /mnt/top/@
if [ -d /mnt/top/@log ]; then
    mv /mnt/top/@log "/mnt/top/@log-old-$TS"
fi
btrfs subvolume create /mnt/top/@log
[ -d /mnt/top/@pkg ] || btrfs subvolume create /mnt/top/@pkg
umount /mnt/top

mount -o "$OPTS,subvol=@" "$ROOT_PART" /mnt
mkdir -p /mnt/home /mnt/var/log /mnt/var/cache/pacman/pkg /mnt/boot
mount -o "$OPTS,subvol=@home" "$ROOT_PART" /mnt/home
mount -o "$OPTS,subvol=@log"  "$ROOT_PART" /mnt/var/log
mount -o "$OPTS,subvol=@pkg"  "$ROOT_PART" /mnt/var/cache/pacman/pkg

# Fresh ESP for the new kernel/bootloader (this is the Linux disk's ESP, not Windows')
mkfs.fat -F 32 "$ESP"
mount "$ESP" /mnt/boot

echo
echo "Done. Old system saved as @old-$TS and @log-old-$TS. @home kept as-is."
findmnt -R /mnt
echo
echo "Next: archinstall --config $(dirname "$0")/config.json"
echo "Create the same user (harry) so it picks up the existing /home/harry"
