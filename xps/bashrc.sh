# GTX 1050 on/off via envycontrol, both reboot straight away
alias gpu-on='sudo envycontrol -s hybrid && systemctl reboot'
alias gpu-off='sudo envycontrol -s integrated && systemctl reboot'
alias gpu='envycontrol -q'

# Battery charge, and health (current max capacity vs design capacity)
alias battery='echo "$(cat /sys/class/power_supply/BAT0/capacity)%"'
alias b='battery'
alias battery-health='echo "$(( $(cat /sys/class/power_supply/BAT0/charge_full) * 100 / $(cat /sys/class/power_supply/BAT0/charge_full_design) ))%"'

# mosh into the desktop's tmux session, reattaching if it exists. The locale is passed
# since mosh-server starts over non-interactive ssh, which skips /etc/locale.conf
alias mdesk='mosh --server="LANG=en_AU.UTF-8 mosh-server" desktop -- tmux new -A -s main'
