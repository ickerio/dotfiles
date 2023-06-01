#!/bin/bash

echo "Preparing to install ickerio/dotfiles"

echo "Installing packages"
pacman -S hyprland hyprpaper waybar kitty dolphin code openssh

echo "Installing yay"
pacman -S --needed git base-devel
git clone https://aur.archlinux.org/yay.git
cd yay
makepkg -si
rm -r yay

echo "Installing AUR packages"
yay -S brave-bin otf-font-awesome

echo "Creating symlinks"
ln -s "$(pwd)hypr" ~/.config/hypr
ln -s "$(pwd)kitty" ~/.config/kitty
ln -s "$(pwd)waybar" ~/.config/waybar
ln -s "$(pwd)wallpaper.png" ~/wallpaper.png

echo "Generating key"
ssh-keygen