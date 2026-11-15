#!/usr/bin/env bash
set -euo pipefail

echo "=========================================================="
echo "  Setting up Arch Linux WSL Development Environment"
echo "  Project: Cosmo Downloader"
echo "=========================================================="

# 1. Initialize Pacman Keyring & Mirrors
echo "[1/7] Initializing pacman keys and system packages..."
pacman-key --init
pacman-key --populate archlinux
pacman -Sy --noconfirm archlinux-keyring
pacman -Syu --noconfirm

# 2. Install Base Development Tools & GitHub CLI
echo "[2/7] Installing base-devel, git, github-cli, and essential tools..."
pacman -S --noconfirm --needed \
    base-devel \
    git \
    github-cli \
    curl \
    wget \
    unzip \
    jq \
    pkgconf \
    clang \
    llvm \
    lld

# 3. Install Rust Toolchain & Components
echo "[3/7] Setting up Rust toolchain via rustup..."
pacman -S --noconfirm --needed rustup
rustup default stable
rustup component add rust-analyzer clippy rustfmt

# 4. Install Node.js & Tooling
echo "[4/7] Installing Node.js and npm..."
pacman -S --noconfirm --needed nodejs npm

# 5. Install Cosmo Downloader GUI & Native Build Dependencies
echo "[5/7] Installing Linux GUI, Audio, and App dependencies (GTK3, eframe, tray-icon, xdo)..."
pacman -S --noconfirm --needed \
    gtk3 \
    libayatana-appindicator \
    openssl \
    sqlite \
    mesa \
    libglvnd \
    libx11 \
    libxcb \
    libxcursor \
    libxrandr \
    libxi \
    libxkbcommon \
    wayland \
    xdotool \
    xclip \
    wl-clipboard \
    alsa-lib

# 6. Setup default non-root user 'steam' with sudo access
echo "[6/7] Configuring non-root user 'steam'..."
if ! id -u steam >/dev/null 2>&1; then
    useradd -m -G wheel -s /bin/bash steam
    echo "steam ALL=(ALL:ALL) NOPASSWD: ALL" > /etc/sudoers.d/steam
    chmod 0440 /etc/sudoers.d/steam
fi

# Ensure rustup is also configured for the steam user
sudo -u steam rustup default stable 2>/dev/null || true
sudo -u steam rustup component add rust-analyzer clippy rustfmt 2>/dev/null || true

# 7. Configure /etc/wsl.conf
echo "[7/7] Configuring /etc/wsl.conf..."
cat << 'EOF' > /etc/wsl.conf
[user]
default = steam

[interop]
enabled = true
appendWindowsPath = true

[boot]
systemd = true
EOF

echo ""
echo "=========================================================="
echo "  Arch Linux WSL environment setup completed successfully!"
echo "=========================================================="
