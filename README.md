# my arch dotfiles

intended for personal use i wouldn't reccomend trying them

## Setup
 - Execute `install.sh`
 - Add git config `git config --global user.name "ickerio"` and `git config --global user.email "private@email.com"`
 - Paste `/home/ickerio/.ssh/id_rsa.pub` on GitHub
 - Setup Brave Sync, login to Spotify


## Notes
 - [Wooting](https://help.wooting.io/article/147-configuring-device-access-for-wootility-under-linux-udev-rules)
 - [XPanel](https://github.com/teamfinalmouse/xpanel-linux-permissions)

# Troubleshooting
 - Wi-Fi not connecting pre install? `ip link set wlan0 up` then `iwctl`
 - Hyprland not launching? `pacman -S mesa-utils mesa` then restart
