#!/usr/bin/env bash
# Build the Root Court kernel, wrap it in a UEFI Limine ISO, and boot QEMU.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "run-qemu.sh must be run from Linux/WSL2" >&2
    exit 1
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/ck-target}"
LIMINE_DIR="${LIMINE_DIR:-$ROOT/third_party/limine}"
BUILD_DIR="$ROOT/build"
ISO="$BUILD_DIR/court-kernel.iso"
OVMF_CODE="${OVMF_CODE:-/usr/share/OVMF/OVMF_CODE_4M.fd}"
OVMF_VARS_SRC="${OVMF_VARS_SRC:-/usr/share/OVMF/OVMF_VARS_4M.fd}"
OVMF_VARS="$BUILD_DIR/ovmf-vars.fd"
KERNEL="$CARGO_TARGET_DIR/x86_64-unknown-none/release/root-court"

mkdir -p "$BUILD_DIR"

if [[ ! -x "$LIMINE_DIR/limine" ]]; then
    echo "fetching Limine v10.x-binary..."
    rm -rf "$LIMINE_DIR"
    git clone https://github.com/limine-bootloader/limine.git --branch=v10.x-binary --depth=1 "$LIMINE_DIR"
    make -C "$LIMINE_DIR"
fi

echo "building root-court (x86_64-unknown-none)..."
cargo build -p root-court --release --target x86_64-unknown-none

ISO_ROOT="$BUILD_DIR/iso_root"
rm -rf "$ISO_ROOT"
mkdir -p "$ISO_ROOT/boot/limine" "$ISO_ROOT/EFI/BOOT"
cp -v "$KERNEL" "$ISO_ROOT/boot/root-court"
cp -v "$ROOT/boot/limine.conf" "$ISO_ROOT/boot/limine/"
cp -v "$LIMINE_DIR/limine-bios.sys" "$LIMINE_DIR/limine-bios-cd.bin" "$LIMINE_DIR/limine-uefi-cd.bin" "$ISO_ROOT/boot/limine/"
cp -v "$LIMINE_DIR/BOOTX64.EFI" "$ISO_ROOT/EFI/BOOT/"

xorriso -as mkisofs -R -r -J \
    -b boot/limine/limine-bios-cd.bin \
    -no-emul-boot -boot-load-size 4 -boot-info-table -hfsplus \
    -apm-block-size 2048 --efi-boot boot/limine/limine-uefi-cd.bin \
    -efi-boot-part --efi-boot-image --protective-msdos-label \
    "$ISO_ROOT" -o "$ISO"
"$LIMINE_DIR/limine" bios-install "$ISO"

cp "$OVMF_VARS_SRC" "$OVMF_VARS"

QEMU=(qemu-system-x86_64
    -M q35
    -cpu max
    -m 256M
    -smp 4
    -serial stdio
    -display none
    -no-reboot
    -device isa-debug-exit,iobase=0xf4,iosize=0x04
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE"
    -drive if=pflash,format=raw,file="$OVMF_VARS"
    -cdrom "$ISO"
)
if [[ -e /dev/kvm && -w /dev/kvm ]]; then
    QEMU+=(-enable-kvm)
fi

echo "booting QEMU..."
set +e
timeout 30s "${QEMU[@]}"
status=$?
set -e

# isa-debug-exit: guest out 0x10 -> qemu exit ((0x10 << 1) | 1) = 33
if [[ "$status" -eq 33 ]]; then
    echo "QEMU: Root Court exited successfully"
    exit 0
fi
echo "QEMU: unexpected exit status $status" >&2
exit 1
