# GTX 1050 on/off via envycontrol, both reboot straight away
alias gpu-on='sudo envycontrol -s hybrid && systemctl reboot'
alias gpu-off='sudo envycontrol -s integrated && systemctl reboot'
alias gpu='envycontrol -q'
