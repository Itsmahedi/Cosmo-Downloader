#!/usr/bin/env bash
set -euo pipefail

FULL=0
MAX_MB=4096

usage() {
    cat <<'EOF'
Usage: clean-workspace.sh [--full] [--max-mb <N>]

Prunes stale Cargo build artifacts, installer staging files, and test temp files.
  --full         Remove the entire target/ directory via `cargo clean`.
  --max-mb <N>   Threshold in MiB above which target/ is fully cleaned (default: 4096).
  -h, --help     Show this help message.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --full)
            FULL=1
            shift
            ;;
        --max-mb)
            if [[ $# -lt 2 || ! "$2" =~ ^[0-9]+$ ]]; then
                echo "Error: --max-mb requires a non-negative integer" >&2
                exit 1
            fi
            MAX_MB="$2"
            shift 2
            ;;
        --max-mb=*)
            val="${1#*=}"
            if [[ ! "$val" =~ ^[0-9]+$ ]]; then
                echo "Error: --max-mb requires a non-negative integer" >&2
                exit 1
            fi
            MAX_MB="$val"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
TARGET_DIR="${PROJECT_ROOT}/target"

dir_size_human() {
    if [[ -d "$1" ]]; then
        du -sh "$1" 2>/dev/null | awk '{print $1}'
    else
        echo "0 (not present)"
    fi
}

dir_size_mb() {
    if [[ -d "$1" ]]; then
        du -sm "$1" 2>/dev/null | awk '{print $1}'
    else
        echo "0"
    fi
}

cd "$PROJECT_ROOT"

echo "=== Cosmo Downloader Workspace Cleanup ==="
echo "Before:"
echo "  target/: $(dir_size_human "$TARGET_DIR")"
df -h / | awk 'NR==1 || NR==2 {print "  " $0}'

TARGET_MB="$(dir_size_mb "$TARGET_DIR")"

if [[ "$FULL" -eq 1 ]]; then
    echo "[*] Full clean requested; running cargo clean..."
    cargo clean
elif [[ "$TARGET_MB" -gt "$MAX_MB" ]]; then
    echo "[*] target/ (${TARGET_MB} MiB) exceeds ${MAX_MB} MiB limit; running full cargo clean..."
    cargo clean
elif [[ -d "$TARGET_DIR" ]]; then
    echo "[*] Pruning workspace crate artifacts (keeping cached dependency builds)..."
    WORKSPACE_PKGS=(
        -p rustget-app
        -p rustget-core
        -p cosmodownloader-cli
        -p cosmodownloader-native-host
    )
    cargo clean "${WORKSPACE_PKGS[@]}"
    if [[ -d "${TARGET_DIR}/x86_64-pc-windows-gnu" ]]; then
        cargo clean --target x86_64-pc-windows-gnu "${WORKSPACE_PKGS[@]}"
    fi
else
    echo "[*] target/ does not exist; skipping cargo clean."
fi

if [[ -d "${PROJECT_ROOT}/installer/dist" ]]; then
    echo "[*] Removing staging directory installer/dist/..."
    rm -rf "${PROJECT_ROOT}/installer/dist"
fi

echo "[*] Removing stale /tmp test and probe artifacts..."
rm -rf \
    /tmp/cosmo_probe_* \
    /tmp/cosmo_test* \
    /tmp/CosmoDownloader_logs \
    /tmp/playwright_chromiumdev_profile-* \
    /tmp/playwright-artifacts-*

if command -v fstrim >/dev/null 2>&1; then
    echo "[*] Running best-effort fstrim /..."
    sudo -n fstrim / 2>/dev/null || true
fi

echo "After:"
echo "  target/: $(dir_size_human "$TARGET_DIR")"
df -h / | awk 'NR==1 || NR==2 {print "  " $0}'
