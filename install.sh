#!/bin/bash

MACHINE=pc

for arg in "$@"; do
    case "$arg" in
        --xps) MACHINE=xps ;;
        *)
            echo "Usage: $0 [--xps]"
            echo "  --xps  install for the Dell XPS 15 9560 (default: desktop PC)"
            exit 1
            ;;
    esac
done

echo "Preparing to install ickerio/dotfiles ($MACHINE)"

if [ "$EUID" -eq 0 ]; then
    echo "Please run as your normal user, sudo is used where needed"
    exit
fi

enable_multilib() {
    # Steam needs the 32-bit multilib repo
    sudo sed -i '/^#\[multilib\]/,/^#Include/ s/^#//' /etc/pacman.conf
    sudo pacman -Syu
}

install_packages() {
    echo "Installing packages"
    sudo pacman -S --needed hyprland hyprpaper foot openssh nano spotify-launcher ttf-jetbrains-mono noto-fonts \
        pipewire-pulse wireplumber playerctl code unzip docker bun prismlauncher fzf zoxide rust

    case "$MACHINE" in
        pc)
            sudo pacman -S --needed nvidia-open nvidia-utils
            ;;
        xps)
            # Intel HD 630 runs the desktop, GTX 1050 via `prime-run` (driver installed from AUR)
            sudo pacman -S --needed linux-headers nvidia-prime mesa vulkan-intel intel-media-driver \
                lib32-mesa lib32-vulkan-intel steam \
                brightnessctl power-profiles-daemon thermald bluez bluez-utils fwupd
            ;;
    esac
}

install_yay() {
    echo "Installing yay"
    sudo pacman -S --needed git base-devel
    # yay-bin is prebuilt, plain yay compiles from source
    git clone https://aur.archlinux.org/yay-bin.git
    cd yay-bin
    makepkg -si
    cd ..
    rm -rf yay-bin
}

install_aur_packages() {
    echo "Installing AUR packages"
    yay -S --needed brave-bin

    if [ "$MACHINE" = xps ]; then
        # GTX 1050 is Pascal, which nvidia-open doesn't support
        yay -S --needed nvidia-580xx-dkms nvidia-580xx-utils lib32-nvidia-580xx-utils envycontrol
    fi
}

install_vscode_extensions() {
    echo "Installing VS Code extensions"
    while read -r extension; do
        code --install-extension "$extension"
    done < vscode/extensions.txt
}

create_udev_rules() {
    sudo cp peripherals/70-wooting.rules /etc/udev/rules.d/
    sudo cp peripherals/70-finalmouse.rules /etc/udev/rules.d/
    sudo rm -f /etc/udev/rules.d/99-finalmouse.rules
    sudo udevadm control --reload-rules && sudo udevadm trigger
}

configure_pc() {
    echo "Configuring desktop PC (NVIDIA)"
    # The kms hook bakes nouveau into the initramfs, so it grabs the GPU before nvidia can
    sudo sed -i '/^HOOKS=/ s/ kms\b//' /etc/mkinitcpio.conf
    sudo mkinitcpio -P
}

configure_xps() {
    echo "Configuring XPS 15 9560"
    sudo cp xps/blacklist.conf /etc/modprobe.d/xps.conf
    sudo mkinitcpio -P
    sudo systemctl enable thermald.service power-profiles-daemon.service bluetooth.service

    # Start with the GTX 1050 off, `gpu-on` / `gpu-off` aliases switch it
    sudo envycontrol -s integrated
    link "$(pwd)/xps/bashrc.sh" ~/.bashrc.local
}

# Replaces old symlinks so this can be re-run on a home kept from a previous install,
# but never overwrites a real file or folder
link() {
    if [ -e "$2" ] && [ ! -L "$2" ]; then
        echo "Skipping $2, it's a real file. Move it and re-run to link it"
        return
    fi
    ln -sfn "$1" "$2"
}

create_symlinks() {
    echo "Symlinking config files"
    mkdir -p ~/.config/"Code - OSS"/User

    ln -sfn "machines/$MACHINE.lua" "$(pwd)/hypr/machine.lua"
    link "$(pwd)/hypr" ~/.config/hypr
    link "$(pwd)/foot" ~/.config/foot
    link "$(pwd)/git" ~/.config/git

    link "$(pwd)/wallpaper.png" ~/wallpaper.png
    link "$(pwd)/vscode/settings.json" ~/.config/"Code - OSS"/User/settings.json
    # Arch's default .bashrc from /etc/skel is fine to replace
    ln -sf "$(pwd)/.bashrc" ~/.bashrc
}

install_cairn() {
    echo "Building cairn"
    cargo build --release --manifest-path cairn/Cargo.toml
    sudo install -Dm755 cairn/target/release/cairn /usr/local/bin/cairn
}

generate_key() {
    # Kept in ~/.ssh across reinstalls
    if ls ~/.ssh/id_* >/dev/null 2>&1; then
        echo "SSH key already exists, skipping"
        return
    fi
    echo "Generating key"
    ssh-keygen
}

if [ "$MACHINE" = xps ]; then
    enable_multilib
fi
install_packages
install_yay
install_aur_packages
install_vscode_extensions
create_udev_rules
if [ "$MACHINE" = pc ]; then
    configure_pc
fi
if [ "$MACHINE" = xps ]; then
    configure_xps
fi
create_symlinks
install_cairn
generate_key

echo "Done. Reboot before running start-hyprland, the new GPU driver only loads on boot"
