#!/bin/bash
# 快速編譯腳本

# 設置環境變量以加速編譯
export CARGO_BUILD_JOBS=6
export RUSTFLAGS="-C link-arg=-Wl,-O1 -C link-arg=-Wl,--as-needed"

# 使用 cargo 環境
source $HOME/.cargo/env

# 清理舊的增量編譯緩存（可選）
# cargo clean -p specs_td

# 編譯選項
case "${1:-dev}" in
    "dev")
        echo "Building in dev mode (fast compile)..."
        cargo build
        ;;
    "quick")
        echo "Building in quick mode (balanced)..."
        cargo build --profile quick
        ;;
    "release")
        echo "Building in release mode (optimized)..."
        cargo build --release
        ;;
    "run")
        echo "Running in dev mode..."
        cargo run
        ;;
    "check")
        echo "Checking code..."
        cargo check
        ;;
    *)
        echo "Usage: $0 [dev|quick|release|run|check]"
        exit 1
        ;;
esac