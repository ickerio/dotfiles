# Switch audio output between the motherboard jack and the USB speakers
alias aux="pactl set-default-sink alsa_output.pci-0000_00_1f.3.analog-stereo"
alias speakers="pactl set-default-sink alsa_output.usb-Audioengine_Ltd._Audioengine_2__ABCDEFB1180003-00.analog-stereo"
