# WnkaUBTM

**WnkaUBTM — Wnka UEFI Boot Manager**

A lightweight and simple UEFI boot manager designed for OS development and experimental operating systems.

WnkaUBTM focuses on three main things:

- **Simplicity** — easy to understand, configure, and integrate.
- **User Experience** — a simple boot menu with keyboard navigation and automatic boot options.
- **Debuggability** — clear boot diagnostics, a safe mode, and a bootloader that returns to the menu instead of crashing when a kernel cannot be loaded.

WnkaUBTM is designed to work not only with **WnkaOS / WnkaU4X**, but also with other OSDev projects that can provide a compatible boot interface.

> **Version 0.0.2 is an early alpha release.** The boot manager itself works, but the interface between WnkaUBTM and kernels is still evolving. See [Known Limitations](#known-limitations).

---

## What's new in 0.0.2

```text
0.0.1  ──────────────────────────────►  0.0.2
```

- **ACPI RSDP is now discovered and validated before the menu.** Hardware Info shows the real RSDP address, and the kernel receives a validated pointer (signature and checksum are checked).
- **The kernel now receives the final memory map**, the one returned by `ExitBootServices()`, instead of a map taken earlier.
- **Safer boot failures.** If a payload is missing, unreadable, or fails ELF validation, WnkaUBTM shows an error and returns to the menu. Autoboot is disabled after a failed attempt, so the same broken image is never started again by the timer.
- **Missing payloads are marked `(MISSING)`** in the menu and are never started by the timer.
- **Stronger ELF validation** (segment bounds, `filesz <= memsz`, overflow checks, unaligned-safe header reads) and **short-read detection** when loading the file.
- **`screen=auto`** is now different from "no `screen` option": `auto` skips the resolution menu and keeps the current GOP mode.
- **`safe_mode`** option for troubleshooting.
- **`debug`** option: boot diagnostics over the COM1 serial port.
- **`default=`** option to choose the default boot entry by name.
- **Advanced Options** submenu: Boot Config, Payload Status, Memory Map, Hardware Info, Reset Screen, Reboot, Shutdown.
- **`no_exit_boot_services`** debug option to keep UEFI Boot Services alive for the kernel.

---

## What does WnkaUBTM do?

WnkaUBTM sits between **UEFI firmware** and the operating system kernel.

```text
                         ┌─────────────────────┐
                         │        UEFI         │
                         └──────────┬──────────┘
                                    │
                                    ▼
                         ┌─────────────────────┐
                         │      WnkaUBTM       │
                         │  Wnka UEFI Boot     │
                         │      Manager        │
                         └──────────┬──────────┘
                                    │
              ┌─────────────────────┼─────────────────────┐
              │                     │                     │
              ▼                     ▼                     ▼
       ┌────────────┐        ┌────────────┐        ┌────────────┐
       │ UEFI Init  │        │ Filesystem │        │    GOP     │
       │ + ACPI     │        │  Scanner   │        │  Manager   │
       └─────┬──────┘        └─────┬──────┘        └─────┬──────┘
             │                     │                     │
             ├── RSDP discovery    ├── boot.cfg          ├── Modes
             └── RSDP validation   └── payload files     ├── Resolution
                                                         └── Framebuffer
             │
             ▼
       ┌────────────┐
       │ Boot Menu  │
       └─────┬──────┘
             │
             ├── ↑ / ↓
             ├── ENTER
             ├── Autoboot / Timeout
             └── Advanced Options
             │
             ▼
       ┌────────────────┐
       │ Resolution     │
       │ Selection      │
       └───────┬────────┘
               │
               ▼
       ┌────────────────┐
       │    ELF64       │
       │    Loader      │
       └───────┬────────┘
               │
               ├── Read file
               ├── Validate ELF
               ├── PT_LOAD
               ├── allocate_pages()
               ├── Copy segments
               └── Zero BSS
               │
               ▼
       ┌────────────────┐
       │  Framebuffer   │
       │  (GOP)         │
       └───────┬────────┘
               │
               ▼
       ┌────────────────┐
       │  Memory Map    │
       │      UEFI      │
       └───────┬────────┘
               │
               ▼
       ┌────────────────────┐
       │  ExitBootServices  │
       │  + final memory    │
       │    map             │
       └─────────┬──────────┘
                 │
                 ▼
       ┌────────────────────┐
       │      BootInfo      │
       ├────────────────────┤
       │ Framebuffer        │
       │ Memory Map         │
       │ ACPI RSDP          │
       │ Boot Services ptr  │
       └─────────┬──────────┘
                 │
                 ▼
       ┌────────────────────┐
       │    Kernel Entry    │
       │   extern "sysv64"  │
       └─────────┬──────────┘
                 │
                 ▼
             ┌───────┐
             │  OS   │
             └───────┘
```

---

## High-Level Architecture

```text
┌─────────────────────────────┐
│          Hardware           │
└──────────────┬──────────────┘
               │
               ▼
┌─────────────────────────────┐
│            UEFI             │
│ Firmware / Boot Services    │
└──────────────┬──────────────┘
               │
               ▼
╔═════════════════════════════╗
║          WnkaUBTM           ║
║                             ║
║  ACPI → FS → Config → Menu  ║
║       ↓                     ║
║  GOP → ELF → Memory Map     ║
║       ↓                     ║
║  ExitBootServices()         ║
║       ↓                     ║
║     BootInfo ABI            ║
╚══════════════╤══════════════╝
               │
               ▼
┌─────────────────────────────┐
│       OS / Kernel           │
│ WnkaU4X / Other OSDev OS    │
└─────────────────────────────┘
```

---

## Features

WnkaUBTM currently provides:

- UEFI boot support (x86_64)
- FAT filesystem scanning
- `boot.cfg` configuration
- Boot menu with keyboard navigation
- Automatic boot with timeout
- `default=` entry selection
- `(MISSING)` markers for payloads that are not on disk
- Advanced Options submenu (Boot Config, Payload Status, Memory Map, Hardware Info, Reset Screen, Reboot, Shutdown)
- GOP video mode selection (`screen=auto`, `screen=WIDTHxHEIGHT`, or an interactive menu)
- Framebuffer initialization
- ELF64 kernel loading with validation
- `PT_LOAD` segment loading
- BSS initialization
- ACPI RSDP discovery with signature and checksum validation
- UEFI memory map retrieval, including the final map after `ExitBootServices()`
- `BootInfo` structure
- `ExitBootServices()` handoff
- SysV64 kernel entry point
- Error handling that returns to the menu
- `safe_mode` for troubleshooting
- COM1 boot diagnostics (`debug=true`)
- `no_exit_boot_services` debug mode

---

## Boot Flow

The general boot process looks like this:

```text
UEFI
 │
 ▼
WnkaUBTM
 │
 ├── Initialize UEFI
 │
 ├── Find and validate ACPI RSDP
 │
 ├── Scan filesystems
 │
 ├── Read and parse boot.cfg
 │
 ├── Initialize COM1 (if debug=true)
 │
 ├── Check which payload files exist
 │
 ├── Show boot menu
 │        │
 │        └── on timeout or ENTER → selected payload
 │
 ├── Select / apply GOP resolution
 │
 ├── Read ELF64 file ──┐
 │                     │  read or validation error:
 ├── Validate ELF ─────┤  show error, return to menu,
 │                     │  autoboot disabled
 │                     ┘
 ├── Load PT_LOAD segments
 │
 ├── Initialize framebuffer (GOP)
 │
 ├── Retrieve UEFI memory map
 │
 ├── ExitBootServices()
 │
 ├── Copy the final memory map
 │
 ├── Build BootInfo
 │
 ▼
Kernel
```

---

# Configuration

WnkaUBTM uses a simple text configuration file named:

`boot.cfg`

The file should be located in the **root of a FAT filesystem**, together with the kernel files.

A minimal configuration can look like this:

```ini
time=5
screen=auto

default=MyOS
MyOS=kernel.elf
```

Rules:

- Lines starting with `#` are comments.
- Empty lines are ignored.
- Option names are case-insensitive.
- Any `name=value` line that is not a known option is a **boot entry**.
- A line without `=` is treated as an entry where the name and the file name are the same (for example, a line containing only `kernel.elf`).
- Up to **9 boot entries** are shown. The configuration file is read up to **2048 bytes**.
- File names are expected to be plain ASCII names located in the root of a FAT volume.

---

## Configuration Options

### `time` / `timeout`

```ini
time=5
```

Sets the boot menu timeout in seconds.

- `time=off` or `time=none` disables the timer.
- `time=0` (or a negative value) also disables the timer.
- If the option is not set, the timeout is **15 seconds**.

Pressing any key in the menu stops the timer. The timer never starts an entry whose file is missing.

---

### `screen`

```ini
screen=auto
screen=1920x1080
```

Selects the screen resolution.

| Value | Behavior |
|-------|----------|
| `auto` | Skip the resolution menu and keep the current GOP mode. |
| `WIDTHxHEIGHT` | Switch to that mode if the firmware offers it. |
| *(not set)* | Show the interactive resolution menu before loading the kernel. |

If the requested `WIDTHxHEIGHT` is not offered by the firmware, the interactive resolution menu is shown. Only modes up to 1920x1080 are listed.

---

### `default`

```ini
default=MyOS
```

Selects the entry that is highlighted when the menu opens (and started by the timer). The value is the **name** of an entry, compared case-insensitively. If the entry does not exist or its file is missing, the first available entry is used.

---

### `debug` / `debug_output`

```ini
debug=true
```

Writes boot diagnostics to the **COM1** serial port (I/O port `0x3F8`, 38400 baud, 8N1). Accepted true values: `true`, `yes`, `on`, `1`.

Serial writes have a bounded wait, so a missing COM1 port will not hang the boot manager.

Useful with QEMU, for example with `-serial stdio`.

---

### `safe_mode`

```ini
safe_mode=true
```

For troubleshooting. In safe mode WnkaUBTM:

- disables the autoboot timer;
- skips the resolution menu and does not change the GOP mode.

---

### `no_exit_boot_services`

```ini
no_exit_boot_services=true
```

Debug option. WnkaUBTM does **not** call `ExitBootServices()` and passes a pointer to the UEFI Boot Services table to the kernel through `BootInfo.boot_services`. Normally (`false`, the default) that pointer is `NULL`.

Several alias spellings are accepted (for example `no_exit_boot_service`, `skip_exit_boot_services`), and the bare name on a line by itself enables the option.

> In this mode the kernel still runs on top of firmware services, so it is not a test of the normal boot path.

---

### Reserved options

`text_mode` and `force_text_mode` are recognized as options (they are not turned into boot entries), but currently have no effect.

---

### Boot Entries

The basic format for an entry is:

```text
Name=filename
```

For example:

```ini
WnkaU4X=wok.elf
MyOS=kernel.elf
WnkaPE=wnka_pe.elf
```

The name on the left is displayed in the boot menu.

The filename on the right specifies the ELF64 executable that WnkaUBTM will load.

If a file does not exist, the entry is shown as `(MISSING)`, it is never started by the timer, and selecting it shows an error instead of booting.

---

## Default Configuration

If no `boot.cfg` is found, WnkaUBTM uses built-in defaults:

```ini
time=15

WnkaU4X=kernel.elf
```

A typical configuration for development with QEMU:

```ini
time=10
screen=auto

# COM1 boot diagnostics: true or false.
debug=true

# Disable autoboot and resolution changes for troubleshooting.
# safe_mode=true

default=WnkaU4X
WnkaU4X=kernel.elf
no_exit_boot_services=false
```

A configuration with several kernels:

```ini
time=10
screen=1366x768

default=MyOS
MyOS=kernel.elf
TestOS=test.elf
AnotherOS=another.elf
```

---

## Why Are `wok.elf` and `wnka_pe.elf` Mentioned?

WnkaUBTM was originally developed as part of the **WnkaU4X** project.

WnkaU4X uses additional executable files:

```text
kernel.elf
wok.elf
wnka_pe.elf
```

The **Payload Status** screen in Advanced Options checks for these three files. They are **not required for every operating system**.

If you are using another OSDev project, simply create your own `boot.cfg` and specify the kernel you want to boot:

```ini
MyOS=kernel.elf
```

---

# Boot Menu

```text
========================================
           WnkaUBTM
========================================

   -> [ WnkaU4X ]
      [ Advanced Options -> ]

 Booting automatically in 10 seconds...
 (Press any arrow key to stop timer)
```

Controls:

| Key | Action |
|-----|--------|
| `↑` / `↓` | Move the selection |
| `ENTER` | Confirm |
| any key | Stop the autoboot timer |

## Advanced Options

```text
========================================
       Advanced Options
========================================

   -> [ Boot Config ]
      [ Payload Status ]
      [ Memory Map ]
      [ Hardware Info ]
      [ Reset Screen ]
      [ Reboot ]
      [ Shutdown ]
      [ <- Back ]
```

| Item | What it shows or does |
|------|-----------------------|
| **Boot Config** | The contents of `boot.cfg`. |
| **Payload Status** | Whether `kernel.elf`, `wok.elf` and `wnka_pe.elf` are `FOUND` or `MISSING`. |
| **Memory Map** | The first entries of the UEFI memory map (type, pages, physical address) and the descriptor size. |
| **Hardware Info** | GOP resolution, stride and framebuffer address, ACPI RSDP address, number of UEFI config tables, total and usable memory. |
| **Reset Screen** | Clears the screen. |
| **Reboot** / **Shutdown** | Resets or powers off through UEFI runtime services. |

---

# Filesystem Layout

A minimal WnkaUBTM setup can look like this:

```text
FAT32
│
├── boot.cfg
├── kernel.elf
└── EFI/
    └── BOOT/
        └── BOOTX64.EFI
```

Where:

```text
BOOTX64.EFI
    │
    └── WnkaUBTM

boot.cfg
    │
    └── Boot configuration

kernel.elf
    │
    └── OS kernel
```

Multiple kernels can also be placed on the same filesystem:

```text
FAT32
│
├── boot.cfg
├── kernel.elf
├── test.elf
├── wok.elf
├── wnka_pe.elf
│
└── EFI/
    └── BOOT/
        └── BOOTX64.EFI
```

---

# Building

WnkaUBTM is written in Rust (`no_std`) using the `uefi` crate (0.28) and `goblin`.

```bash
cd bootloader
cargo build --release --target x86_64-unknown-uefi
```

Copy the resulting `.efi` file to `EFI/BOOT/BOOTX64.EFI` on a FAT32 partition, and put `boot.cfg` and your kernel files in the root of the same partition.

---

# Kernel Handoff

Before transferring control to the operating system, WnkaUBTM prepares a `BootInfo` structure containing information required by the kernel.

Conceptually:

```text
BootInfo
│
├── Framebuffer
│   ├── Base address
│   ├── Width
│   ├── Height
│   └── Pixels per scan line
│
├── UEFI Memory Map
│   ├── Pointer
│   ├── Size in bytes
│   └── Descriptor size
│
├── ACPI RSDP
│
└── Boot Services pointer (NULL unless no_exit_boot_services=true)
```

After the required information is prepared, WnkaUBTM calls:

```text
ExitBootServices()
```

and transfers control to the kernel entry point.

The kernel is entered using the:

```text
extern "sysv64"
```

calling convention, with a pointer to `BootInfo` as the first argument.

## BootInfo layout

All fields are 64-bit on x86_64 and the structure is `#[repr(C)]`.

Rust:

```rust
#[repr(C)]
pub struct FramebufferInfo {
    pub base_address: u64,
    pub width: u64,
    pub height: u64,
    pub pixels_per_scan_line: u64,
}

#[repr(C)]
pub struct BootInfo {
    pub framebuffer: FramebufferInfo,
    pub memory_map: *const u8,
    pub memory_map_size: usize,   // size in bytes
    pub descriptor_size: usize,   // stride between descriptors
    pub rsdp: u64,                // 0 if no valid RSDP was found
    pub boot_services: *const core::ffi::c_void, // NULL after ExitBootServices
}
```

C:

```c
#include <stdint.h>

typedef struct {
    uint64_t base_address;
    uint64_t width;
    uint64_t height;
    uint64_t pixels_per_scan_line;
} FramebufferInfo;

typedef struct {
    FramebufferInfo framebuffer;
    const uint8_t  *memory_map;
    uint64_t        memory_map_size;  /* bytes */
    uint64_t        descriptor_size;  /* stride */
    uint64_t        rsdp;
    const void     *boot_services;    /* NULL unless no_exit_boot_services=true */
} BootInfo;

__attribute__((sysv_abi)) void kernel_main(const BootInfo *boot_info);
```

## Notes for kernel developers

- **Walk the memory map using `descriptor_size`** as the stride, never `sizeof` of your own structure. Each descriptor follows the standard UEFI memory descriptor layout (type, physical start, virtual start, number of pages, attributes).
- **Copy `BootInfo` immediately.** It lives on the boot manager's stack, and the memory map buffer is allocated as loader data. Do not reuse that memory until you have copied what you need.
- **Segments are loaded at their physical addresses** (`p_paddr`). Make sure your linker script sets them, that they are page-aligned, and that they do not overlap firmware-reserved memory.
- The kernel is entered with the **firmware's page tables and stack**. Set up your own GDT, IDT, paging and stack as early as possible.
- `rsdp` is a **validated** pointer (signature and checksum have been checked). ACPI 2.0 is preferred over ACPI 1.0.
- `boot_services` is `NULL` in the normal boot path. A kernel must not require it unless it is deliberately run with `no_exit_boot_services=true`.
- The framebuffer is cleared to black before the handoff. The **pixel format is not passed yet** (see Roadmap).

---

# Compatibility

WnkaUBTM was created for **OSDev projects**, especially experimental x86_64 operating systems using UEFI.

It can be used with **WnkaU4X** and other kernels as long as they implement the required boot interface and are compatible with the `BootInfo` structure provided by WnkaUBTM.

The goal is not to force an operating system to use a particular kernel architecture.

Instead, WnkaUBTM provides a simple bridge:

```text
UEFI
  ↓
WnkaUBTM
  ↓
BootInfo
  ↓
Your Kernel
```

---

# Project Structure

The WnkaUBTM codebase currently looks like this:

```text
WnkaUBTM
│
├── bootloader/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
│
└── README.md
```

The exact structure may change as the project develops.

---

# ELF64 Loading

WnkaUBTM can load ELF64 kernels (up to 16 MiB).

The loader processes:

```text
Read file (short reads are detected)
      │
      ▼
ELF Header
      │
      ▼
Program Headers
      │
      ▼
PT_LOAD segments
      │
      ├── Allocate pages
      │
      ├── Copy segment data
      │
      └── Zero BSS
      │
      ▼
Kernel Entry Point
```

## ELF validation

Before anything is copied into memory, WnkaUBTM checks:

- ELF magic, 64-bit class, little-endian encoding, x86_64 machine type;
- a non-null entry point and the presence of program headers;
- that the program header table fits inside the file;
- for every `PT_LOAD` segment: the file range stays inside the image, offsets do not overflow, `p_filesz <= p_memsz`, the physical address is non-zero, and the memory size is non-zero.

Program headers are read with unaligned-safe reads.

If validation fails, the error is shown and WnkaUBTM returns to the menu with autoboot disabled.

---

# GOP / Graphics

WnkaUBTM uses the **UEFI Graphics Output Protocol (GOP)** to configure the framebuffer.

The boot manager can:

- Enumerate available GOP modes (up to 1920x1080)
- Select a resolution from `boot.cfg` or from an interactive menu
- Keep the current mode with `screen=auto`
- Initialize the framebuffer
- Pass framebuffer information to the kernel

The framebuffer information is then available through `BootInfo`.

Conceptually:

```text
UEFI GOP
   │
   ▼
WnkaUBTM
   │
   ├── Resolution
   ├── Width
   ├── Height
   ├── Pixels per scan line
   └── Framebuffer
   │
   ▼
BootInfo
   │
   ▼
Kernel
```

---

# ACPI

WnkaUBTM searches the UEFI configuration tables for the **ACPI RSDP** before the menu is shown.

- The ACPI 2.0+ table is preferred; ACPI 1.0 is used as a fallback.
- The `"RSD PTR "` signature is checked.
- The checksum of the first 20 bytes is checked, and for ACPI 2.0+ the extended checksum is checked as well.

```text
UEFI
 │
 ▼
ACPI Configuration Tables
 │
 ▼
RSDP Discovery
 │
 ▼
Signature + Checksum validation
 │
 ▼
BootInfo
 │
 ▼
Kernel
```

The address is shown in **Advanced Options → Hardware Info** and passed to the kernel through `BootInfo`. If no valid RSDP is found, `rsdp` is `0`.

This allows the kernel to continue ACPI initialization after leaving UEFI Boot Services.

---

# Memory Map

WnkaUBTM retrieves the UEFI memory map and hands it to the kernel.

The memory map contains information about the physical memory regions available to the operating system.

`ExitBootServices()` can change the memory map, so after the call WnkaUBTM copies the **final** memory map into the buffer that `BootInfo` points to.

```text
UEFI Memory Map
       │
       ▼
ExitBootServices()
       │
       ▼
  Final Memory Map
       │
       ▼
    BootInfo
       │
       ▼
     Kernel
       │
       ▼
Physical Memory Manager
```

Always use `BootInfo.descriptor_size` as the stride when walking the map.

---

# ExitBootServices

After all required UEFI information has been collected, WnkaUBTM calls:

```text
ExitBootServices()
```

At this point, control over the machine is transferred from the UEFI boot environment to the operating system.

The final handoff is approximately:

```text
UEFI Boot Services
        │
        ▼
    WnkaUBTM
        │
        ├── Load Kernel
        ├── Setup GOP
        ├── Find ACPI
        ├── Get Memory Map
        │
        ▼
ExitBootServices()
        │
        ├── Copy final Memory Map
        └── Build BootInfo
        │
        ▼
     Kernel
```

With `no_exit_boot_services=true` this step is skipped and the Boot Services pointer is passed to the kernel instead.

---

# Diagnostics

With `debug=true`, WnkaUBTM writes short messages to COM1. Example:

```text
[BOOT] WnkaUBTM starting
[ACPI] RSDP signature and checksum valid before menu
[CONFIG] entry found: kernel.elf
[BOOT] selected payload: kernel.elf
[BOOT] reading payload
[ELF] header validated
[MEMORY] using final post-exit memory map
```

Error messages use the same prefixes, for example `[ERROR] ELF validation failed`.

## Troubleshooting

| Symptom | What to check |
|---------|---------------|
| An entry shows `(MISSING)` | The file name in `boot.cfg` must match a file in the **root** of a FAT volume. |
| Menu returns after "Could not boot ..." | Read the reason on screen. Typical causes: truncated file, wrong architecture, invalid segment. |
| Resolution menu appears on every boot | Set `screen=auto` or a valid `WIDTHxHEIGHT`. |
| Something hangs during boot | Set `safe_mode=true` to disable the timer and resolution changes. |
| No serial output | Check `debug=true` and that the machine or emulator has a COM1 port. |
| Hardware Info shows `RSDP : not found` | The firmware did not provide a valid RSDP in its configuration tables. |

---

# Known Limitations

WnkaUBTM 0.0.2 is an early alpha. Known limitations:

- x86_64 UEFI only.
- Only the **root directory** of a FAT volume is used, and file names should be ASCII.
- Files are searched on **every FAT volume**, and the first match is used. This is not necessarily the volume WnkaUBTM was started from.
- If a segment's physical address is not page-aligned, or its memory cannot be allocated, WnkaUBTM stops with an error instead of returning to the menu.
- WnkaUBTM does not check that the entry point lies inside a loaded segment.
- The pixel format of the framebuffer (RGB/BGR) is **not** passed to the kernel yet.
- **Payload Status** checks only the three WnkaU4X file names (`kernel.elf`, `wok.elf`, `wnka_pe.elf`), not the entries from `boot.cfg`.
- Secure Boot is not supported.
- Tested mostly in QEMU/OVMF. Testing on real hardware is limited.

---

# Compatibility Philosophy

WnkaUBTM is intended to be a **general-purpose OSDev boot manager**, rather than a bootloader tied exclusively to one operating system.

The basic idea is:

```text
                    ┌─────────────┐
                    │    UEFI     │
                    └──────┬──────┘
                           │
                           ▼
                    ┌─────────────┐
                    │  WnkaUBTM   │
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
              ▼            ▼            ▼
          WnkaU4X        MyOS       Other OSDev
              │            │            │
              └────────────┼────────────┘
                           │
                           ▼
                         Kernel
```

As long as an operating system can work with the boot interface provided by WnkaUBTM, it can potentially be loaded by the boot manager.

---

# Project Status

**Current version: `0.0.2` (alpha)**

WnkaUBTM is currently under active development.

The project is expected to evolve together with future versions of WnkaU4X.

Features, configuration syntax, the boot interface, and internal architecture may change in future releases.

---

# Roadmap

Planned improvements may include:

- Pass the framebuffer pixel format in `BootInfo`
- Load files from the volume WnkaUBTM was started from
- Return to the menu on every ELF loading failure, not only on validation errors
- Check that the entry point lies inside a loaded segment
- Payload Status based on the entries from `boot.cfg`
- More flexible boot configuration
- Additional boot options
- Improved hardware compatibility and testing on real machines
- More boot manager customization
- Additional filesystem support
- Improved boot menu UI
- Subdirectory paths in `boot.cfg`
- Better boot failure diagnostics

The roadmap may change as development continues.

---

# Philosophy

WnkaUBTM is built around a simple idea:

> **A boot manager should be simple to configure, simple to understand, and pleasant to use.**

It should provide the functionality an OS developer needs without making the boot process unnecessarily complicated.

```text
Simple configuration
        +
Simple boot flow
        +
Useful UEFI features
        +
Good user experience
        =
             WnkaUBTM
```

---

# License

See the `LICENSE` file for the license used by this project.

---

# Thanks for Reading!

WnkaUBTM will continue to evolve alongside future versions of the project.

Thank you for checking out **WnkaUBTM**!

**Wnka UEFI Boot Manager — simple booting for OSDev.**
