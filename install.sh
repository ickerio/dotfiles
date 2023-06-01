#!/bin/bash

echo "Preparing to install ickerio/dotfiles"

echo "Installing packages"
sudo pacman -S hyprland hyprpaper kitty nautilus code openssh

echo "Installing yay"
pacman -S --needed git base-devel
git clone https://aur.archlinux.org/yay.git
cd yay
makepkg -si
rm -r yay

echo "Installing AUR packages"
yay -S waybar-hyprland-git brave-bin otf-font-awesome

echo "Creating symlinks"
ln -s "$(pwd)/hypr" ~/.config/hypr
ln -s "$(pwd)/kitty" ~/.config/kitty
ln -s "$(pwd)/waybar" ~/.config/waybar
ln -s "$(pwd)/wallpaper.png" ~/wallpaper.png

ln -s "$(pwd)/Nordic/gtk-3.0" ~/.config/gtk-3.0
ln -s "$(pwd)/Nordic/gtk-4.0" ~/.config/gtk-4.0

echo "Generating key"
ssh-keygen