#!/usr/bin/env bash
set -euo pipefail

# Find Windows Desktop path
WINDOWS_USER="steam"
DESKTOP_DIR="/mnt/c/Users/${WINDOWS_USER}/OneDrive/Desktop"
if [ ! -d "${DESKTOP_DIR}" ]; then
    DESKTOP_DIR="/mnt/c/Users/${WINDOWS_USER}/Desktop"
fi

if [ ! -d "${DESKTOP_DIR}" ]; then
    echo "Error: Windows Desktop directory not found."
    exit 1
fi

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${PROJECT_ROOT}/target"
DIST_DIR="${PROJECT_ROOT}/dist"

echo "=== Copy Newest Cosmo Downloader Build to Desktop ==="
echo "Target directory: ${TARGET_DIR}"
echo "Desktop directory: ${DESKTOP_DIR}"

FOUND_FILES=()

# Search for Windows binaries in target
for PDIR in "${TARGET_DIR}/x86_64-pc-windows-gnu/release" "${TARGET_DIR}/x86_64-pc-windows-gnu/debug" "${TARGET_DIR}/release" "${TARGET_DIR}/debug"; do
    if [ -d "${PDIR}" ]; then
        if [ -f "${PDIR}/CosmoDownloader.exe" ]; then
            FOUND_FILES+=("${PDIR}/CosmoDownloader.exe")
        fi
    fi
done

if [ ${#FOUND_FILES[@]} -eq 0 ]; then
    echo "No compiled Windows application binary (CosmoDownloader.exe) found in ${TARGET_DIR}."
    echo "Please build the Windows project first: cargo build --release --target x86_64-pc-windows-gnu"
    exit 1
fi

# Find the newest file
NEWEST_FILE=""
NEWEST_TIME=0

for f in "${FOUND_FILES[@]}"; do
    MTIME=$(stat -c %Y "$f" 2>/dev/null || stat -f %m "$f" 2>/dev/null || echo 0)
    if [ "$MTIME" -ge "$NEWEST_TIME" ]; then
        NEWEST_TIME="$MTIME"
        NEWEST_FILE="$f"
    fi
done

BASENAME=$(basename "${NEWEST_FILE}")
DEST="${DESKTOP_DIR}/${BASENAME}"

echo "Found newest build: ${NEWEST_FILE}"
echo "Copying to Desktop: ${DEST}"
if [ -f "${DEST}" ]; then
    mv -f "${DEST}" "${DEST}.old" 2>/dev/null || true
fi
cp -f "${NEWEST_FILE}" "${DEST}"
rm -f "${DEST}.old" 2>/dev/null || true
chmod 755 "${DEST}" 2>/dev/null || true

FILE_SIZE=$(ls -lh "${DEST}" | awk '{print $5}')
echo "Successfully copied ${BASENAME} (${FILE_SIZE}) to Windows Desktop!"
