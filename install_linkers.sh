#!/bin/bash
# 安裝快速連結器腳本

echo "Installing fast linkers for Rust..."

# 檢測系統類型
if [ -f /etc/debian_version ]; then
    # Debian/Ubuntu 系統
    echo "Detected Debian/Ubuntu system"
    
    # 更新套件列表
    sudo apt update
    
    # 安裝 lld (LLVM 連結器)
    echo "Installing lld..."
    sudo apt install -y lld clang
    
    # 安裝 mold (最新最快的連結器)
    echo "Installing mold..."
    # 方法 1: 從 apt 安裝（如果可用）
    if apt-cache search mold | grep -q "mold"; then
        sudo apt install -y mold
    else
        # 方法 2: 從 GitHub 下載預編譯版本
        echo "Downloading mold from GitHub..."
        MOLD_VERSION="2.4.0"  # 請檢查最新版本
        wget https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-x86_64-linux.tar.gz
        tar xzf mold-${MOLD_VERSION}-x86_64-linux.tar.gz
        sudo cp -r mold-${MOLD_VERSION}-x86_64-linux/* /usr/local/
        rm -rf mold-${MOLD_VERSION}-x86_64-linux*
    fi
    
elif [ -f /etc/fedora-release ] || [ -f /etc/redhat-release ]; then
    # Fedora/RHEL 系統
    echo "Detected Fedora/RHEL system"
    
    # 安裝 lld
    sudo dnf install -y lld clang
    
    # 安裝 mold
    sudo dnf install -y mold
    
elif [ -f /etc/arch-release ]; then
    # Arch Linux 系統
    echo "Detected Arch Linux system"
    
    # 安裝 lld
    sudo pacman -S --noconfirm lld clang
    
    # 安裝 mold
    sudo pacman -S --noconfirm mold
    
else
    echo "Unknown system type"
    echo "Please install lld and mold manually"
    exit 1
fi

echo ""
echo "Installation complete!"
echo ""
echo "To use lld, uncomment the lld section in .cargo/config.toml"
echo "To use mold, uncomment the mold section in .cargo/config.toml"
echo ""
echo "Benchmark results (typical):"
echo "- Default linker: 100% (baseline)"
echo "- lld: ~50% faster"
echo "- mold: ~70-80% faster"
echo ""
echo "Testing installations..."
which lld && echo "✓ lld installed"
which ld.lld && echo "✓ ld.lld installed"
which mold && echo "✓ mold installed"