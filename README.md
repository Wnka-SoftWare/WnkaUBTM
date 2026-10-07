# WnkaUBTM

**WnkaUBTM — Wnka UEFI Boot Manager**

A lightweight and simple UEFI boot manager designed for OS development and experimental operating systems.

WnkaUBTM focuses on two main things:

- **Simplicity** — easy to understand, configure, and integrate.
- **User Experience** — a simple boot menu with keyboard navigation and automatic boot options.

WnkaUBTM is designed to work not only with **WnkaOS / WnkaU4X**, but also with other OSDev projects that can provide a compatible boot interface.

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
       │            │        │  Scanner   │        │  Manager   │
       └─────┬──────┘        └─────┬──────┘        └─────┬──────┘
             │                     │                     │
             │                     ├── boot.cfg          ├── Modes
             │                     └── kernel.elf        ├── Resolution
             │                                           └── Framebuffer
             │
             ▼
       ┌────────────┐
       │ Boot Menu  │
       └─────┬──────┘
             │
             ├── ↑ / ↓
             ├── ENTER
             └── Autorun / Timeout
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
               ├── ELF Header
               ├── Program Headers
               ├── PT_LOAD
               ├── allocate_pages()
               ├── Copy segments
               └── Zero BSS
               │
               ▼
       ┌────────────────┐
       │      ACPI      │
       │ RSDP Discovery │
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
       │      BootInfo      │
       ├────────────────────┤
       │ Framebuffer        │
       │ Memory Map         │
       │ ACPI RSDP          │
       └─────────┬──────────┘
                 │
                 ▼
       ┌────────────────────┐
       │  ExitBootServices  │
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
║  FS → Config → Menu → GOP   ║
║       ↓                     ║
║  ELF → ACPI → Memory Map    ║
║       ↓                     ║
║     BootInfo ABI            ║
║       ↓                     ║
║  ExitBootServices()         ║
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

- UEFI boot support
- FAT filesystem scanning
- `boot.cfg` configuration
- Boot menu
- Keyboard navigation
- Automatic boot with timeout
- `autorun` entries
- GOP video mode selection
- Resolution selection
- Framebuffer initialization
- ELF64 kernel loading
- `PT_LOAD` segment loading
- BSS initialization
- ACPI RSDP discovery
- UEFI memory map retrieval
- `BootInfo` structure
- `ExitBootServices()` handoff
- SysV64 kernel entry point

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
 ├── Scan filesystems
 │
 ├── Read boot.cfg
 │
 ├── Find available OS entries
 │
 ├── Show boot menu
 │
 ├── Select GOP resolution
 │
 ├── Load ELF64 kernel
 │
 ├── Find ACPI RSDP
 │
 ├── Retrieve UEFI memory map
 │
 ├── Build BootInfo
 │
 ├── ExitBootServices()
 │
 ▼
Kernel
```

---

# Configuration

WnkaUBTM uses a simple text configuration file named:

`boot.cfg`

The file should be located on a FAT filesystem together with the kernel files.

A minimal configuration can look like this:

```ini
time=5
screen=1920x1080

OS=kernel.elf <- autorun
```

---

## Configuration Options

### `time`

```ini
time=5
```

Sets the boot menu timeout to **5 seconds**.

This option is optional.

---

### `screen`

```ini
screen=1920x1080
```

Specifies the preferred screen resolution.

This option is optional.

---

### OS Entries

The basic format for an OS entry is:

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

The filename on the right specifies the ELF executable that WnkaUBTM will load.

---

## Autorun

An entry can be marked as the default boot entry using:

```text
<- autorun
```

For example:

```ini
WnkaU4X=wok.elf <- autorun
MyOS=kernel.elf
WnkaPE=wnka_pe.elf
```

In this example, `WnkaU4X` will be selected automatically when the boot menu timeout expires.

The `autorun` option is optional.

---

## Default Configuration

The default configuration used by WnkaUBTM is:

```ini
time=5
screen=1920x1080

WnkaU4X=wok.elf <- autorun
OS=kernel.elf
WnkaPE=wnka_pe.elf
```

You can modify this configuration or create your own entries.

For example:

```ini
time=10
screen=1366x768

MyOS=kernel.elf <- autorun
TestOS=test.elf
AnotherOS=another.elf
```

---

## Why Are `wok.elf` and `wnka_pe.elf` Included?

WnkaUBTM was originally developed as part of the **WnkaU4X** project.

WnkaU4X uses two additional executable files:

```text
wok.elf
wnka_pe.elf
```

These files are therefore included in the default WnkaUBTM configuration.

They are **not required for every operating system**.

If you are using another OSDev project, you can simply create your own `boot.cfg` and specify the kernel you want to boot.

For example:

```ini
MyOS=kernel.elf <- autorun
```

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
│
└── ACPI RSDP
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

calling convention.

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

The WnkaUBTM codebase is divided into several components:

```text
WnkaUBTM
│
├── bootloader/
│   └── uefi/
│       └──  main.rs
│
│
│
└── README.md
```

The exact structure may change as the project develops.

---

# Boot Interface

WnkaUBTM passes information to the kernel through a `BootInfo` structure.

The interface currently provides information about:

- Framebuffer
- UEFI memory map
- ACPI RSDP
- Memory map descriptor size
- Memory map size

Conceptually:

```text
┌─────────────────────────────┐
│          BootInfo           │
├─────────────────────────────┤
│ Framebuffer Information     │
│                             │
│ Memory Map                  │
│                             │
│ ACPI RSDP                   │
└─────────────────────────────┘
```

This allows the kernel to start without having to perform the initial UEFI discovery itself.

---

# ELF64 Loading

WnkaUBTM can load ELF64 kernels.

The loader processes:

```text
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

The loaded kernel is then started using its ELF entry point.

---

# GOP / Graphics

WnkaUBTM uses the **UEFI Graphics Output Protocol (GOP)** to configure the framebuffer.

The boot manager can:

- Enumerate available GOP modes
- Select a resolution
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

WnkaUBTM searches for the **ACPI RSDP** provided by UEFI firmware.

The RSDP address is passed to the kernel through `BootInfo`.

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
BootInfo
 │
 ▼
Kernel
```

This allows the kernel to continue ACPI initialization after leaving UEFI Boot Services.

---

# Memory Map

Before handing control to the kernel, WnkaUBTM retrieves the UEFI memory map.

The memory map contains information about the physical memory regions available to the operating system.

The relevant information is passed to the kernel through `BootInfo`.

```text
UEFI Memory Map
       │
       ▼
    WnkaUBTM
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
        └── Build BootInfo
        │
        ▼
ExitBootServices()
        │
        ▼
     Kernel
```

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

**Current version: `0.0.1`**

WnkaUBTM is currently under active development.

The project is expected to evolve together with future versions of WnkaUBTM.

Features, configuration syntax, the boot interface, and internal architecture may change in future releases.

---

# Roadmap

Planned improvements may include:

- More robust ELF validation
- Better framebuffer information
- More flexible boot configuration
- Additional boot options
- Improved hardware compatibility
- Better error handling
- More boot manager customization
- Additional filesystem support
- Improved boot menu UI
- Improved configuration parsing
- Better boot failure handling
- More flexible kernel loading

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
