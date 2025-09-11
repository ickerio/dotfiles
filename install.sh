#!/bin/bash

echo "Preparing to install ickerio/dotfiles"

if [ "$EUID" -ne 0 ]; then
    echo "Please run as root"
    exit
fi

install_packages() {
    echo "Installing packages"
    sudo pacman -S nvidia-open nvidia-utils hyprland hyprpaper openssh nano spotify-launcher unzip ttf-jetbrains-mono
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

install_bun() {
    curl -fsSL https://bun.sh/install | bash
}

install_aur_packages() {
    echo "Installing AUR packages"
    yay -S cursor-bin brave-bin aylurs-gtk-shell
}

create_udev_rules() {
    sudo cp peripherals/70-wooting.rules /etc/udev/rules.d/
    sudo cp peripherals/99-finalmouse.rules /etc/udev/rules.d/
    sudo udevadm control --reload-rules && sudo udevadm trigger
}

create_symlinks() {
    echo "Symlinking config files"
    ln -s "$(pwd)/hypr" ~/.config/hypr
    ln -s "$(pwd)/foot" ~/.config/foot

    ln -s "$(pwd)/wallpaper.png" ~/wallpaper.png
    ln -s "$(pwd)/cursor/settings.json" ~/.config/Cursor/User/settings.json
    ln -s "$(pwd)/.bashrc" ~/.bashrc
}

generate_key() {
    echo "Generating key"
    ssh-keygen
}

install_packages
install_yay
install_bun
install_aur_packages
create_udev_rules
create_symlinks
generate_key