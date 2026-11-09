# GTX 1050 on/off via envycontrol, both reboot straight away
alias gpu-on='sudo envycontrol -s hybrid && systemctl reboot'
alias gpu-off='sudo envycontrol -s integrated && systemctl reboot'
alias gpu='envycontrol -q'

# Battery health (current max capacity vs design capacity)
alias battery-health='echo "$(( $(cat /sys/class/power_supply/BAT0/charge_full) * 100 / $(cat /sys/class/power_supply/BAT0/charge_full_design) ))%"'

# Toggle the Bose headphones/speaker connection, addresses kept out of this public repo in ~/.bashrc.private
[[ -f ~/.bashrc.private ]] && . ~/.bashrc.private
_bt_toggle() {
    if bluetoothctl info "$1" | grep -q 'Connected: yes'; then
        bluetoothctl disconnect "$1"
    else
        bluetoothctl connect "$1"
    fi
}
headphones() { _bt_toggle "${BOSE_MAC:?set BOSE_MAC in ~/.bashrc.private}"; }
speaker() { _bt_toggle "${SPEAKER_MAC:?set SPEAKER_MAC in ~/.bashrc.private}"; }
