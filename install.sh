#!/bin/bash

echo "Preparing to install ickerio/dotfiles"

if [ "$EUID" -ne 0 ]; then
    echo "Please run as root"
    exit
fi

install_packages() {
    echo "Installing packages"
    sudo pacman -S nvidia-open nvidia-utils hyprland hyprpaper openssh nano
}

enable_bluetooth() {
    echo "Enabling bluetooth"
    sudo pacman -S bluez bluez-utils
    sudo systemctl enable bluetooth.service
}

install_yay() {
    echo "Installing yay"
    pacman -S --needed git base-devel
    git clone https://aur.archlinux.org/yay.git
    cd yay
    makepkg -si
    cd ..
    rm -r yay
}

install_aur_packages() {
    echo "Installing AUR packages"
    yay -S cursor-bin brave-bin aylurs-gtk-shell
}

create_udev_rules() {
    sudo cp udev/70-wooting.rules /etc/udev/rules.d/
    sudo cp udev/99-finalmouse.rules /etc/udev/rules.d/
    sudo udevadm control --reload-rules && sudo udevadm trigger
}

create_symlinks() {
    echo "Creating symlinks"
    ln -s "$(pwd)/hypr" ~/.config/hypr
    ln -s "$(pwd)/wallpaper.png" ~/wallpaper.png
    ln -s "$(pwd)/cursor/settings.json" ~/.config/Cursor/User/settings.json
}

generate_key() {
    echo "Generating key"
    ssh-keygen
}

install_packages
#enable_bluetooth
install_yay
install_aur_packages
create_udev_rules
create_symlinks
generate_key