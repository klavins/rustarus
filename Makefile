EFI_BINARY = target/x86_64-unknown-uefi/release/rustarus.efi
IMG = rustarus-uefi.img
DISK = disk.img

OVMF_CODE = /opt/homebrew/share/qemu/edk2-x86_64-code.fd
OVMF_VARS = /opt/homebrew/share/qemu/edk2-i386-vars.fd

QEMU_COMMON = qemu-system-x86_64 \
	-drive if=pflash,format=raw,readonly=on,file=$(OVMF_CODE) \
	-drive if=pflash,format=raw,file=/tmp/ovmf-vars.fd \
	-drive format=raw,file=$(IMG),if=virtio \
	-drive file=$(DISK),format=raw,if=none,id=data -device ide-hd,drive=data,bus=ide.0 \
	-m 256 -smp 1 -net none \
	-display cocoa,zoom-to-fit=on -full-screen \
	-machine pcspk-audiodev=snd \
	-audiodev coreaudio,id=snd

.PHONY: build img run run-bga run-vmware clean

build:
	cargo build --release

img: build
	dd if=/dev/zero of=$(IMG) bs=1M count=64 2>/dev/null
	mformat -i $(IMG) -F ::
	mmd -i $(IMG) ::/EFI
	mmd -i $(IMG) ::/EFI/BOOT
	mcopy -i $(IMG) $(EFI_BINARY) ::/EFI/BOOT/BOOTX64.EFI

$(DISK):
	dd if=/dev/zero of=$(DISK) bs=1M count=1 2>/dev/null
	./util/idu -f $(DISK) format

run: img $(DISK)
	cp $(OVMF_VARS) /tmp/ovmf-vars.fd
	$(QEMU_COMMON) -vga none -device ramfb

run-bga: img $(DISK)
	cp $(OVMF_VARS) /tmp/ovmf-vars.fd
	$(QEMU_COMMON) -vga none -device VGA,vgamem_mb=16

run-vmware: img $(DISK)
	cp $(OVMF_VARS) /tmp/ovmf-vars.fd
	$(QEMU_COMMON) -vga none -device vmware-svga,vgamem_mb=16

clean:
	cargo clean
	rm -f $(IMG)
