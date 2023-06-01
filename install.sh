#!/bin/bash

echo "Preparing to install ickerio/dotfiles"

if [ "$EUID" -ne 0 ]; then
    echo "Please run as root"
    exit
fi

install_packages() {
    echo "Installing packages"
    sudo pacman -S hyprland hyprpaper kitty nautilus code openssh nano
}

install_laptop_gpu_power_saving() {
    echo "Laptop GPU power saving"
    sudo pacman -S bbswitch bumblebee
    sudo systemctl enable bumblebeed.service
}

install_yay() {
    echo "Installing yay"
    pacman -S --needed git base-devel
    git clone https://aur.archlinux.org/yay.git
    cd yay
    makepkg -si
    rm -r yay
}

install_aur_packages() {
    echo "Installing AUR packages"
    yay -S waybar-hyprland-git brave-bin otf-font-awesome
}

create_symlinks() {
    echo "Creating symlinks"
    ln -s "$(pwd)/hypr" ~/.config/hypr
    ln -s "$(pwd)/kitty" ~/.config/kitty
    ln -s "$(pwd)/waybar" ~/.config/waybar
    ln -s "$(pwd)/wallpaper.png" ~/wallpaper.png

    ln -s "$(pwd)/Nordic/gtk-3.0" ~/.config/gtk-3.0
    ln -s "$(pwd)/Nordic/gtk-4.0" ~/.config/gtk-4.0
}

generate_key() {
    echo "Generating key"
    ssh-keygen
}

# Check if "laptop" flag was passed
if [[ "$1" == "laptop" ]]; then
    install_laptop_gpu_power_saving
fi

install_packages
install_yay
install_aur_packages
create_symlinks
generate_key