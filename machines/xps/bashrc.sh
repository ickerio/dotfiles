# GTX 1050 on/off via envycontrol, both reboot straight away
alias gpu-on='sudo envycontrol -s hybrid && systemctl reboot'
alias gpu-off='sudo envycontrol -s integrated && systemctl reboot'
alias gpu='envycontrol -q'

# Battery health (current max capacity vs design capacity)
alias battery-health='echo "$(( $(cat /sys/class/power_supply/BAT0/charge_full) * 100 / $(cat /sys/class/power_supply/BAT0/charge_full_design) ))%"'

# mosh into a tmux session on the desktop (default "default"), creating it if needed.
# Each call attaches a grouped view (default-1, default-2...) so several foot windows share the
# same tmux windows but switch between them independently; a view is destroyed once detached.
# The locale is passed since mosh-server starts over non-interactive ssh, which skips /etc/locale.conf
mdesk() {
    mosh --server="LANG=en_AU.UTF-8 mosh-server" desktop -- sh -c \
        'tmux has -t "=$1" 2>/dev/null || tmux new -d -s "$1"; exec tmux new -t "$1" \; set destroy-unattached on' \
        sh "${1:-default}"
}

# Connect the Bose headphones, address kept out of this public repo in ~/.bashrc.private
[[ -f ~/.bashrc.private ]] && . ~/.bashrc.private
bose() { bluetoothctl connect "${BOSE_MAC:?set BOSE_MAC in ~/.bashrc.private}"; }
