#![no_std]
#![no_main]

extern crate alloc;

use uefi::allocator::Allocator;

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

use uefi::CStr16;
use core::panic::PanicInfo;
use core::fmt::Write;
use goblin::elf::header::{header64::Header, EM_X86_64};
use goblin::elf::program_header::program_header64::ProgramHeader;
use goblin::elf::program_header::PT_LOAD;
use uefi::cstr16;
use uefi::prelude::*;
use uefi::Identify;
use uefi::proto::console::gop::GraphicsOutput;
use uefi::proto::console::text::{Key, ScanCode};
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, FileType};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::table::boot::{AllocateType, MemoryDescriptor, MemoryType, SearchType, OpenProtocolParams, OpenProtocolAttributes};
use uefi::table::cfg::{ACPI_GUID, ACPI2_GUID};
use uefi::table::runtime::ResetType;

const MAX_KERNEL_SIZE: usize = 16 * 1024 * 1024;
const READ_CHUNK: usize = 65536;
const SERIAL_COM1: u16 = 0x3f8;

unsafe fn serial_out(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
}

unsafe fn serial_in(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
    value
}

fn init_debug_serial(enabled: bool) {
    if !enabled {
        return;
    }

    unsafe {
        serial_out(SERIAL_COM1 + 1, 0x00);
        serial_out(SERIAL_COM1 + 3, 0x80);
        serial_out(SERIAL_COM1, 0x03);
        serial_out(SERIAL_COM1 + 1, 0x00);
        serial_out(SERIAL_COM1 + 3, 0x03);
        serial_out(SERIAL_COM1 + 2, 0xc7);
        serial_out(SERIAL_COM1 + 4, 0x0b);
    }
}

fn debug_log(enabled: bool, message: &str) {
    if !enabled {
        return;
    }

    for byte in message.bytes() {
        unsafe {
            let mut ready = false;
            for _ in 0..100_000 {
                if serial_in(SERIAL_COM1 + 5) & 0x20 != 0 {
                    ready = true;
                    break;
                }
            }
            if !ready {
                return;
            }
            serial_out(SERIAL_COM1, byte);
        }
    }
}

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
    pub memory_map_size: usize,
    pub descriptor_size: usize,
    pub rsdp: u64,
    pub boot_services: *const core::ffi::c_void,
}

type KernelEntryPoint = extern "sysv64" fn(boot_info: *const BootInfo) -> !;

#[derive(Copy, Clone, PartialEq)]
enum EntryAction {
    LoadKernel,
    Submenu,
    Reboot,
    Shutdown,
    Back,
    HardwareInfo,
    BootConfig,
    PayloadStatus,
    MemoryMap,
    ResetScreen,
}

#[derive(Copy, Clone)]
struct BootEntry<'a> {
    name: &'a str,
    filename: &'a str,
    is_autorun: bool,
    action: EntryAction,
}

fn to_utf16(s: &str, buf: &mut [u16]) -> usize {
    let mut len = 0;
    for b in s.bytes() {
        if len >= buf.len() - 1 { break; }
        buf[len] = b as u16;
        len += 1;
    }
    buf[len] = 0;
    len
}

fn validate_elf_header(kernel_buffer_addr: u64, kernel_size: usize, header: &Header) -> Result<(), &'static str> {
    if &header.e_ident[0..4] != b"\x7fELF" {
        return Err("Invalid ELF magic");
    }

    if header.e_ident[4] != 2 {
        return Err("Unsupported ELF class: expected ELF64");
    }

    if header.e_ident[5] != 1 {
        return Err("Unsupported ELF encoding: expected little-endian");
    }

    if header.e_machine != EM_X86_64 {
        return Err("Unsupported ELF machine type: expected x86_64");
    }

    if header.e_entry == 0 {
        return Err("ELF entry point is null");
    }

    if header.e_phoff == 0 || header.e_phnum == 0 {
        return Err("ELF has no program headers");
    }

    let phdr_size = header.e_phentsize as usize;
    if phdr_size < core::mem::size_of::<ProgramHeader>() {
        return Err("ELF program header size is too small");
    }

    let kernel_size_u64 = kernel_size as u64;
    let phdr_table_end = header
        .e_phoff
        .checked_add((header.e_phnum as u64).saturating_mul(header.e_phentsize as u64))
        .ok_or("Program header table overflow")?;

    if phdr_table_end > kernel_size_u64 {
        return Err("Program header table extends past end of kernel image");
    }

    let phdr_offset = header.e_phoff as usize;
    let phdr_count = header.e_phnum as usize;
    for i in 0..phdr_count {
        let phdr_ptr = (kernel_buffer_addr as usize + phdr_offset + i * phdr_size) as *const ProgramHeader;
        let phdr = unsafe { core::ptr::read_unaligned(phdr_ptr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        if phdr.p_offset as u64 > kernel_size_u64 {
            return Err("ELF load segment offset exceeds kernel size");
        }

        let segment_end = phdr
            .p_offset
            .checked_add(phdr.p_filesz)
            .ok_or("ELF segment file size overflow")?;
        if segment_end > kernel_size_u64 {
            return Err("ELF load segment extends past end of kernel image");
        }

        if phdr.p_filesz > phdr.p_memsz {
            return Err("ELF segment filesz > memsz");
        }

        if phdr.p_paddr == 0 {
            return Err("ELF load segment has invalid physical address");
        }

        if phdr.p_memsz == 0 {
            return Err("ELF load segment has zero memory size");
        }
    }

    Ok(())
}

fn checksum_is_zero(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

unsafe fn validate_rsdp(address: u64) -> Option<u8> {
    if address == 0 {
        return None;
    }

    let rsdp = address as *const u8;
    let signature = unsafe { core::slice::from_raw_parts(rsdp, 8) };
    if signature != b"RSD PTR " {
        return None;
    }

    let legacy = unsafe { core::slice::from_raw_parts(rsdp, 20) };
    if !checksum_is_zero(legacy) {
        return None;
    }

    let revision = unsafe { *rsdp.add(15) };
    if revision < 2 {
        return Some(1);
    }

    let length = unsafe { core::ptr::read_unaligned(rsdp.add(20).cast::<u32>()) } as usize;
    if !(36..=4096).contains(&length) {
        return None;
    }
    let extended = unsafe { core::slice::from_raw_parts(rsdp, length) };
    checksum_is_zero(extended).then_some(2)
}

fn find_rsdp(config_tables: &[uefi::table::cfg::ConfigTableEntry]) -> (u64, u8) {
    let mut legacy_rsdp = (0, 0);

    for entry in config_tables {
        let address = entry.address as u64;
        if entry.guid == ACPI2_GUID {
            if let Some(revision) = unsafe { validate_rsdp(address) } {
                return (address, revision);
            }
        } else if entry.guid == ACPI_GUID && legacy_rsdp.0 == 0 {
            if let Some(revision) = unsafe { validate_rsdp(address) } {
                legacy_rsdp = (address, revision);
            }
        }
    }

    legacy_rsdp
}

fn halt_with_error(system_table: &mut SystemTable<Boot>, msg: &str) -> ! {
    let _ = writeln!(system_table.stdout(), "");
    let _ = writeln!(system_table.stdout(), "[ERROR] {}", msg);
    let _ = writeln!(system_table.stdout(), "");
    let _ = writeln!(system_table.stdout(), "System halted.");
    loop {
        system_table.boot_services().stall(1_000_000);
    }
}

fn show_load_error(system_table: &mut SystemTable<Boot>, filename: &str, reason: &str) {
    let _ = writeln!(system_table.stdout(), "");
    let _ = writeln!(system_table.stdout(), "[ERROR] Could not boot {}", filename);
    let _ = writeln!(system_table.stdout(), "        {}", reason);
    let _ = writeln!(system_table.stdout(), "Press any key to return to the boot menu...");

    let _ = system_table.stdin().reset(false);
    loop {
        if let Ok(Some(_)) = system_table.stdin().read_key() {
            break;
        }
        system_table.boot_services().stall(10_000);
    }
}

fn read_boot_config(bt: &uefi::table::boot::BootServices, cfg_buffer: &mut [u8]) -> usize {
    let fs_handles = match bt.locate_handle_buffer(SearchType::ByProtocol(&SimpleFileSystem::GUID)) {
        Ok(handles) => handles,
        Err(_) => return 0,
    };

    for &handle in fs_handles.iter() {
        if let Ok(mut fs) = bt.open_protocol_exclusive::<SimpleFileSystem>(handle) {
            if let Ok(mut root_dir) = fs.open_volume() {
                if let Ok(file) = root_dir.open(cstr16!("boot.cfg"), FileMode::Read, FileAttribute::empty()) {
                    if let Ok(FileType::Regular(mut reg)) = file.into_type() {
                        if let Ok(len) = reg.read(cfg_buffer) {
                            if len > 0 {
                                return len;
                            }
                        }
                    }
                }
            }
        }
    }

    0
}

fn detect_payloads(bt: &uefi::table::boot::BootServices) -> (bool, bool, bool) {
    let fs_handles = match bt.locate_handle_buffer(SearchType::ByProtocol(&SimpleFileSystem::GUID)) {
        Ok(handles) => handles,
        Err(_) => return (false, false, false),
    };

    let mut has_unknown_os = false;
    let mut has_kernel = false;
    let mut has_pe = false;

    for &handle in fs_handles.iter() {
        let Ok(mut fs) = bt.open_protocol_exclusive::<SimpleFileSystem>(handle) else {
            continue;
        };
        let Ok(mut root_dir) = fs.open_volume() else {
            continue;
        };

        has_unknown_os |= root_dir
            .open(cstr16!("kernel.elf"), FileMode::Read, FileAttribute::empty())
            .is_ok();
        has_kernel |= root_dir
            .open(cstr16!("wok.elf"), FileMode::Read, FileAttribute::empty())
            .is_ok();
        has_pe |= root_dir
            .open(cstr16!("wnka_pe.elf"), FileMode::Read, FileAttribute::empty())
            .is_ok();

        if has_unknown_os && has_kernel && has_pe {
            break;
        }
    }

    (has_unknown_os, has_kernel, has_pe)
}

fn payload_exists(bt: &uefi::table::boot::BootServices, filename: &str) -> bool {
    let mut path_buffer = [0u16; 64];
    let path_len = to_utf16(filename, &mut path_buffer);
    let Ok(path) = CStr16::from_u16_with_nul(&path_buffer[..=path_len]) else {
        return false;
    };

    let Ok(fs_handles) = bt.locate_handle_buffer(SearchType::ByProtocol(&SimpleFileSystem::GUID)) else {
        return false;
    };

    for &handle in fs_handles.iter() {
        let Ok(mut fs) = bt.open_protocol_exclusive::<SimpleFileSystem>(handle) else {
            continue;
        };
        let Ok(mut root_dir) = fs.open_volume() else {
            continue;
        };
        if let Ok(file) = root_dir.open(path, FileMode::Read, FileAttribute::empty()) {
            if matches!(file.into_type(), Ok(FileType::Regular(_))) {
                return true;
            }
        }
    }

    false
}

fn show_boot_config(system_table: &mut SystemTable<Boot>, cfg_buffer: &[u8], cfg_len: usize) {
    let _ = system_table.stdout().clear();
    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "            Boot Configuration");
    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "");

    if cfg_len == 0 {
        let _ = writeln!(system_table.stdout(), "No boot.cfg found.");
    } else {
        let text = core::str::from_utf8(&cfg_buffer[..cfg_len]).unwrap_or("<invalid UTF-8>");
        for line in text.lines() {
            let _ = writeln!(system_table.stdout(), "{}", line);
        }
    }

    let _ = writeln!(system_table.stdout(), "");
    let _ = writeln!(system_table.stdout(), "Press any key to return...");

    let _ = system_table.stdin().reset(false);
    loop {
        if let Ok(Some(_)) = system_table.stdin().read_key() {
            break;
        }
        system_table.boot_services().stall(10_000);
    }
}

#[derive(Clone, Copy)]
struct BootConfig<'a> {
    timeout: Option<isize>,
    resolution: Option<(usize, usize)>,
    screen_auto: bool,
    default_entry: Option<&'a str>,
    no_exit_boot_services: bool,
    debug: bool,
    safe_mode: bool,
}

fn is_no_exit_boot_services_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("no_exit_boot_servise")
        || key.eq_ignore_ascii_case("no_exit_boot_service")
        || key.eq_ignore_ascii_case("no_exit_boot_services")
        || key.eq_ignore_ascii_case("no_boot_service_exit")
        || key.eq_ignore_ascii_case("no_boot_services_exit")
        || key.eq_ignore_ascii_case("nobootserviseexit")
        || key.eq_ignore_ascii_case("skip_exit_boot_services")
}

fn parse_boot_config(cfg_bytes: &[u8]) -> BootConfig<'_> {
    let mut config = BootConfig {
        timeout: Some(15),
        resolution: None,
        screen_auto: false,
        default_entry: None,
        no_exit_boot_services: false,
        debug: false,
        safe_mode: false,
    };
    let Ok(text) = core::str::from_utf8(cfg_bytes) else {
        return config;
    };

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if is_no_exit_boot_services_key(line) {
            config.no_exit_boot_services = true;
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        if key.eq_ignore_ascii_case("time") || key.eq_ignore_ascii_case("timeout") {
            if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("none") {
                config.timeout = None;
            } else if let Ok(seconds) = value.parse::<isize>() {
                config.timeout = if seconds > 0 { Some(seconds) } else { None };
            }
        } else if key.eq_ignore_ascii_case("screen") {
            if value.eq_ignore_ascii_case("auto") {
                config.resolution = None;
                config.screen_auto = true;
            } else if let Some((width, height)) = value
                .split_once('x')
                .or_else(|| value.split_once('X'))
            {
                if let (Ok(width), Ok(height)) = (width.trim().parse::<usize>(), height.trim().parse::<usize>()) {
                    if width > 0 && height > 0 {
                        config.resolution = Some((width, height));
                        config.screen_auto = false;
                    }
                }
            }
        } else if key.eq_ignore_ascii_case("default") && !value.is_empty() {
            config.default_entry = Some(value);
        } else if key.eq_ignore_ascii_case("debug") || key.eq_ignore_ascii_case("debug_output") {
            config.debug = value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
                || value == "1";
        } else if key.eq_ignore_ascii_case("safe_mode") {
            config.safe_mode = value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
                || value == "1";
        } else if is_no_exit_boot_services_key(key) {
            if value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
                || value == "1"
            {
                config.no_exit_boot_services = true;
            } else if value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("no")
                || value.eq_ignore_ascii_case("off")
                || value == "0"
            {
                config.no_exit_boot_services = false;
            }
        }
    }

    config
}

fn show_payload_status(system_table: &mut SystemTable<Boot>, has_unknown_os: bool, has_kernel: bool, has_pe: bool) {
    let _ = system_table.stdout().clear();
    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "             Payload Status");
    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "");

    let _ = writeln!(system_table.stdout(), " Unknown OS payload : {}", if has_unknown_os { "FOUND" } else { "MISSING" });
    let _ = writeln!(system_table.stdout(), " Wnka kernel        : {}", if has_kernel { "FOUND" } else { "MISSING" });
    let _ = writeln!(system_table.stdout(), " PE image           : {}", if has_pe { "FOUND" } else { "MISSING" });

    let _ = writeln!(system_table.stdout(), "");
    let _ = writeln!(system_table.stdout(), "Press any key to return...");

    let _ = system_table.stdin().reset(false);
    loop {
        if let Ok(Some(_)) = system_table.stdin().read_key() {
            break;
        }
        system_table.boot_services().stall(10_000);
    }
}

fn show_memory_map(system_table: &mut SystemTable<Boot>) {
    let _ = system_table.stdout().clear();
    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "              Memory Map");
    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "");

    let mmap_sizes = system_table.boot_services().memory_map_size();
    let desc_size = mmap_sizes.entry_size;
    let buf_size = mmap_sizes.map_size + 8 * desc_size;
    let pages = (buf_size + 4095) / 4096;

    if let Ok(addr) = system_table.boot_services().allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages) {
        let slice = unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, buf_size) };
        if let Ok(mmap) = system_table.boot_services().memory_map(slice) {
            let _ = writeln!(system_table.stdout(), " Map entries : {}", mmap.entries().count());
            let _ = writeln!(system_table.stdout(), " Entry size  : {} bytes", desc_size);
            let _ = writeln!(system_table.stdout(), "");

            let mut idx = 0usize;
            for desc in mmap.entries() {
                let _ = writeln!(system_table.stdout(), " [{:02}] type={} pages={} phys=0x{:x}", idx, desc.ty.0, desc.page_count, desc.phys_start);
                idx += 1;
                if idx >= 16 {
                    break;
                }
            }
        } else {
            let _ = writeln!(system_table.stdout(), "Failed to read memory map.");
        }
    } else {
        let _ = writeln!(system_table.stdout(), "Unable to allocate temporary map buffer.");
    }

    let _ = writeln!(system_table.stdout(), "");
    let _ = writeln!(system_table.stdout(), "Press any key to return...");

    let _ = system_table.stdin().reset(false);
    loop {
        if let Ok(Some(_)) = system_table.stdin().read_key() {
            break;
        }
        system_table.boot_services().stall(10_000);
    }
}

fn show_hardware_info(system_table: &mut SystemTable<Boot>, rsdp_addr: u64) {
    let _ = system_table.stdout().clear();
    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "         Hardware Information");
    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "");

    // === GOP / Framebuffer: сначала собираем данные, потом выводим ===
    let mut gop_found = false;
    let mut gop_w: u64 = 0;
    let mut gop_h: u64 = 0;
    let mut gop_stride: u64 = 0;
    let mut gop_fb: u64 = 0;

    {
        if let Ok(gop_handle) = system_table.boot_services().get_handle_for_protocol::<GraphicsOutput>() {
            let params = OpenProtocolParams {
                handle: gop_handle,
                agent: system_table.boot_services().image_handle(),
                controller: None,
            };
            if let Ok(mut gop) = unsafe {
                system_table.boot_services().open_protocol::<GraphicsOutput>(
                    params,
                    OpenProtocolAttributes::GetProtocol,
                )
            } {
                let info = gop.current_mode_info();
                let (w, h) = info.resolution();
                let stride = info.stride();
                let fb = gop.frame_buffer().as_mut_ptr() as u64;

                gop_w = w as u64;
                gop_h = h as u64;
                gop_stride = stride as u64;
                gop_fb = fb;
                gop_found = true;
            }
        }
    } // ← gop закрыт, system_table свободен

    if gop_found {
        let _ = writeln!(system_table.stdout(), " [Graphics]");
        let _ = writeln!(system_table.stdout(), "   Resolution : {}x{}", gop_w, gop_h);
        let _ = writeln!(system_table.stdout(), "   Stride     : {}", gop_stride);
        let _ = writeln!(system_table.stdout(), "   Framebuffer: 0x{:x}", gop_fb);
    } else {
        let _ = writeln!(system_table.stdout(), " [Graphics] GOP not available");
    }

    let _ = writeln!(system_table.stdout(), "");

    // === ACPI ===
    let _ = writeln!(system_table.stdout(), " [ACPI]");
    if rsdp_addr != 0 {
        let _ = writeln!(system_table.stdout(), "   RSDP       : 0x{:x}", rsdp_addr);
    } else {
        let _ = writeln!(system_table.stdout(), "   RSDP       : not found");
    }

    let _ = writeln!(system_table.stdout(), "");

    // === UEFI / Memory ===
    let _ = writeln!(system_table.stdout(), " [UEFI]");
    let config_tables = system_table.config_table().len();
    let _ = writeln!(system_table.stdout(), "   Config tables: {}", config_tables);
    let _ = writeln!(system_table.stdout(), "   Boot services: active");

    let _ = writeln!(system_table.stdout(), "");
    let _ = writeln!(system_table.stdout(), " [Memory]");
    let mmap_sizes = system_table.boot_services().memory_map_size();
    let _ = writeln!(system_table.stdout(), "   Map size   : {} bytes", mmap_sizes.map_size);
    let _ = writeln!(system_table.stdout(), "   Entry size : {} bytes", mmap_sizes.entry_size);

    let desc_size = mmap_sizes.entry_size;
    let buf_size = mmap_sizes.map_size + 8 * desc_size;
    let pages = (buf_size + 4095) / 4096;

    let mut total_mb: u64 = 0;
    let mut usable_mb: u64 = 0;
    let mut mem_ok = false;

    if let Ok(addr) = system_table.boot_services().allocate_pages(
        AllocateType::AnyPages,
        MemoryType::LOADER_DATA,
        pages,
    ) {
        let slice = unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, buf_size) };
        if let Ok(mmap) = system_table.boot_services().memory_map(slice) {
            let mut total_pages: u64 = 0;
            let mut usable_pages: u64 = 0;

            for desc in mmap.entries() {
                total_pages += desc.page_count;
                // EFI_CONVENTIONAL_MEMORY = 7
                if desc.ty.0 == 7 {
                    usable_pages += desc.page_count;
                }
            }

            total_mb = (total_pages * 4) / 1024;
            usable_mb = (usable_pages * 4) / 1024;
            mem_ok = true;
        }
    }

    if mem_ok {
        let _ = writeln!(system_table.stdout(), "   Total      : ~{} MB", total_mb);
        let _ = writeln!(system_table.stdout(), "   Usable     : ~{} MB", usable_mb);
    } else {
        let _ = writeln!(system_table.stdout(), "   Failed to read memory map");
    }

    let _ = writeln!(system_table.stdout(), "");
    let _ = writeln!(system_table.stdout(), "Press any key to return...");

    let _ = system_table.stdin().reset(false);
    loop {
        if let Ok(Some(_)) = system_table.stdin().read_key() {
            break;
        }
        system_table.boot_services().stall(10_000);
    }
}

#[entry]
fn main(_image_handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    if uefi::helpers::init(&mut system_table).is_err() {
        let _ = writeln!(system_table.stdout(), "Failed to initialize utilities");
        loop { system_table.boot_services().stall(1_000_000); }
    }

    let _ = system_table.stdout().clear();
    let _ = system_table.boot_services().set_watchdog_timer(0, 0x10000, None);

    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(system_table.stdout(), "WnkaUBTM 0.0.2");

    let mut fb_info = FramebufferInfo {
        base_address: 0,
        width: 0,
        height: 0,
        pixels_per_scan_line: 0,
    };
    let (rsdp_addr, acpi_kind) = find_rsdp(system_table.config_table());
    let (has_unknown_os, has_kernel, has_pe) = {
        let bt = system_table.boot_services();
        detect_payloads(bt)
    };
    let mut cfg_buffer = [0u8; 2048];
    let cfg_len = {
        let bt = system_table.boot_services();
        read_boot_config(bt, &mut cfg_buffer)
    };

    let boot_config = parse_boot_config(&cfg_buffer[..cfg_len]);
    init_debug_serial(boot_config.debug);
    debug_log(boot_config.debug, "[BOOT] WnkaUBTM starting\r\n");
    if rsdp_addr != 0 {
        debug_log(boot_config.debug, "[ACPI] RSDP signature and checksum valid before menu\r\n");
    } else {
        debug_log(boot_config.debug, "[ACPI] no valid RSDP found\r\n");
    }
    if cfg_len == 0 {
        let _ = writeln!(system_table.stdout(), "[INFO] boot.cfg not found; using defaults.");
    }

    let mut boot_attempt_failed = false;
    let (kernel_buffer_addr, entry_point_addr, phdr_offset, phdr_size, phdr_count) = 'boot_attempt: loop {
    let mut parsed_timeout = if boot_config.safe_mode || boot_attempt_failed { None } else { boot_config.timeout };
    let parsed_res = if boot_config.safe_mode { None } else { boot_config.resolution };

    // === Главное меню ===
    let mut main_entries: [BootEntry; 10] = [BootEntry {
        name: "",
        filename: "",
        is_autorun: false,
        action: EntryAction::LoadKernel,
    }; 10];
    let mut main_count = 0;
    let mut entry_available = [false; 9];

    if cfg_len > 0 {
        if let Ok(cfg_str) = core::str::from_utf8(&cfg_buffer[..cfg_len]) {
            for line in cfg_str.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                if let Some((name, value)) = line.split_once('=') {
                    let name = name.trim();
                    let filename = value.trim();
                    let is_setting = name.eq_ignore_ascii_case("time")
                        || name.eq_ignore_ascii_case("timeout")
                        || name.eq_ignore_ascii_case("screen")
                        || name.eq_ignore_ascii_case("default")
                        || is_no_exit_boot_services_key(name)
                        || name.eq_ignore_ascii_case("debug")
                        || name.eq_ignore_ascii_case("debug_output")
                        || name.eq_ignore_ascii_case("safe_mode")
                        || name.eq_ignore_ascii_case("text_mode")
                        || name.eq_ignore_ascii_case("force_text_mode");

                    if !is_setting && !name.is_empty() && !filename.is_empty() && main_count < 9 {
                        main_entries[main_count] = BootEntry {
                            name,
                            filename,
                            is_autorun: false,
                            action: EntryAction::LoadKernel,
                        };
                        main_count += 1;
                    }
                    continue;
                }

                if is_no_exit_boot_services_key(line) {
                    continue;
                }

                if main_count < 9 {
                    main_entries[main_count] = BootEntry {
                        name: line,
                        filename: line,
                        is_autorun: false,
                        action: EntryAction::LoadKernel,
                    };
                    main_count += 1;
                }
            }
        }
    }

    if main_count == 0 {
        main_entries[main_count] = BootEntry {
            name: "WnkaU4X",
            filename: "kernel.elf",
            is_autorun: true,
            action: EntryAction::LoadKernel,
        };
        main_count += 1;
    }

    // Fallback, если boot.cfg нет
    if main_count == 0 {
        if has_unknown_os {
            main_entries[main_count] = BootEntry {
                name: "Untitled OS",
                filename: "kernel.elf",
                is_autorun: false,
                action: EntryAction::LoadKernel,
            };
            main_count += 1;
        }
        if has_kernel {
            main_entries[main_count] = BootEntry {
                name: "WnkaU4X",
                filename: "wok.elf",
                is_autorun: false,
                action: EntryAction::LoadKernel,
            };
            main_count += 1;
        }
        if has_pe {
            main_entries[main_count] = BootEntry {
                name: "WnkaU4X (Install)",
                filename: "wnka_pe.elf",
                is_autorun: false,
                action: EntryAction::LoadKernel,
            };
            main_count += 1;
        }
    }

    for i in 0..main_count {
        entry_available[i] = {
            let bt = system_table.boot_services();
            payload_exists(bt, main_entries[i].filename)
        };
        if entry_available[i] {
            debug_log(boot_config.debug, "[CONFIG] entry found: ");
        } else {
            debug_log(boot_config.debug, "[CONFIG] entry missing: ");
        }
        debug_log(boot_config.debug, main_entries[i].filename);
        debug_log(boot_config.debug, "\r\n");
    }
    let payload_count = main_count;

    // Добавляем Advanced Options
    if main_count < 10 {
        main_entries[main_count] = BootEntry {
            name: "Advanced Options ->",
            filename: "",
            is_autorun: false,
            action: EntryAction::Submenu,
        };
        main_count += 1;
    }

    if main_count == 0 {
        halt_with_error(&mut system_table, "No bootable payloads found!\n        Make sure boot.cfg exists or\n        kernel.elf / wok.elf / wnka_pe.elf is present.");
    }

    // === Advanced меню ===
    let advanced_entries: [BootEntry; 8] = [
        BootEntry { name: "Boot Config",      filename: "", is_autorun: false, action: EntryAction::BootConfig },
        BootEntry { name: "Payload Status",  filename: "", is_autorun: false, action: EntryAction::PayloadStatus },
        BootEntry { name: "Memory Map",       filename: "", is_autorun: false, action: EntryAction::MemoryMap },
        BootEntry { name: "Hardware Info",    filename: "", is_autorun: false, action: EntryAction::HardwareInfo },
        BootEntry { name: "Reset Screen",     filename: "", is_autorun: false, action: EntryAction::ResetScreen },
        BootEntry { name: "Reboot",           filename: "", is_autorun: false, action: EntryAction::Reboot },
        BootEntry { name: "Shutdown",         filename: "", is_autorun: false, action: EntryAction::Shutdown },
        BootEntry { name: "<- Back",          filename: "", is_autorun: false, action: EntryAction::Back },
    ];
    let advanced_count = 8;

    // === Цикл меню ===
    let mut current_is_main = true;
    let mut selected = 0usize;
    let mut default_entry_found = false;

    for i in 0..payload_count {
        if entry_available[i] {
            selected = i;
            break;
        }
    }

    if let Some(default_entry) = boot_config.default_entry {
        for i in 0..payload_count {
            if entry_available[i] && main_entries[i].name.eq_ignore_ascii_case(default_entry) {
                selected = i;
                default_entry_found = true;
                break;
            }
        }
    }

    if !default_entry_found {
        for i in 0..main_count {
            if i < payload_count && entry_available[i] && main_entries[i].is_autorun {
                selected = i;
                break;
            }
        }
    }

    let mut redraw = true;
    let mut timeout = parsed_timeout.unwrap_or(-1);
    let mut ticks = 0;
    let _ = system_table.stdin().reset(false);

    // target_filename будет присвоена при выходе из цикла
    let target_filename: &str;

    'menu_loop: loop {
        let (entries, count) = if current_is_main {
            (&main_entries[..], main_count)
        } else {
            (&advanced_entries[..], advanced_count)
        };

        if redraw {
            let _ = system_table.stdout().clear();
            let _ = writeln!(system_table.stdout(), "========================================");
            if current_is_main {
                let _ = writeln!(system_table.stdout(), "           WnkaUBTM");
            } else {
                let _ = writeln!(system_table.stdout(), "       Advanced Options");
            }
            let _ = writeln!(system_table.stdout(), "========================================");
            let _ = writeln!(system_table.stdout(), "");

            for i in 0..count {
                if i == selected {
                    if current_is_main && i < payload_count && !entry_available[i] {
                        let _ = writeln!(system_table.stdout(), "   -> [ {} ] (MISSING)", entries[i].name);
                    } else {
                        let _ = writeln!(system_table.stdout(), "   -> [ {} ]", entries[i].name);
                    }
                } else {
                    if current_is_main && i < payload_count && !entry_available[i] {
                        let _ = writeln!(system_table.stdout(), "      [ {} ] (MISSING)", entries[i].name);
                    } else {
                        let _ = writeln!(system_table.stdout(), "      [ {} ]", entries[i].name);
                    }
                }
            }

            let _ = writeln!(system_table.stdout(), "");
            if current_is_main
                && selected < payload_count
                && entry_available[selected]
                && parsed_timeout.is_some()
                && timeout > 0
            {
                let _ = writeln!(system_table.stdout(), " Booting automatically in {} seconds...", timeout);
                let _ = writeln!(system_table.stdout(), " (Press any arrow key to stop timer)");
            } else {
                let _ = writeln!(system_table.stdout(), " Use UP/DOWN to select, ENTER to confirm.");
            }
            redraw = false;
        }

        let mut key_pressed = false;
        if let Ok(Some(key)) = system_table.stdin().read_key() {
            key_pressed = true;
            parsed_timeout = None;

            match key {
                Key::Special(ScanCode::UP) => {
                    if selected > 0 { selected -= 1; }
                }
                Key::Special(ScanCode::DOWN) => {
                    if selected < count - 1 { selected += 1; }
                }
                Key::Printable(c) => {
                    let code = u16::from(c);
                    if code == 13 || code == 10 {
                        // Enter
                        let chosen = &entries[selected];

                        match chosen.action {
                            EntryAction::LoadKernel => {
                                if current_is_main && selected < payload_count && !entry_available[selected] {
                                    debug_log(boot_config.debug, "[ERROR] selected payload is missing\r\n");
                                    show_load_error(&mut system_table, chosen.filename, "File was not found during config validation.");
                                    redraw = true;
                                    continue 'menu_loop;
                                }
                                debug_log(boot_config.debug, "[BOOT] selected payload: ");
                                debug_log(boot_config.debug, chosen.filename);
                                debug_log(boot_config.debug, "\r\n");
                                target_filename = chosen.filename;
                                break 'menu_loop;
                            }
                            EntryAction::HardwareInfo => {
                                show_hardware_info(&mut system_table, rsdp_addr);
                                redraw = true;
                                continue 'menu_loop;
                            }
                            EntryAction::BootConfig => {
                                show_boot_config(&mut system_table, &cfg_buffer[..cfg_len], cfg_len);
                                redraw = true;
                                continue 'menu_loop;
                            }
                            EntryAction::PayloadStatus => {
                                show_payload_status(&mut system_table, has_unknown_os, has_kernel, has_pe);
                                redraw = true;
                                continue 'menu_loop;
                            }
                            EntryAction::MemoryMap => {
                                show_memory_map(&mut system_table);
                                redraw = true;
                                continue 'menu_loop;
                            }
                            EntryAction::ResetScreen => {
                                let _ = system_table.stdout().clear();
                                redraw = true;
                                continue 'menu_loop;
                            }
                            EntryAction::Submenu => {
                                current_is_main = false;
                                selected = 0;
                                redraw = true;
                                continue 'menu_loop;
                            }
                            EntryAction::Back => {
                                current_is_main = true;
                                selected = 0;
                                redraw = true;
                                continue 'menu_loop;
                            }
                            EntryAction::Reboot => {
                                let _ = writeln!(system_table.stdout(), "\nRebooting...");
                                system_table.runtime_services().reset(ResetType::COLD, Status::SUCCESS, None);
                            }
                            EntryAction::Shutdown => {
                                let _ = writeln!(system_table.stdout(), "\nShutting down...");
                                system_table.runtime_services().reset(ResetType::SHUTDOWN, Status::SUCCESS, None);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if key_pressed {
            redraw = true;
            continue;
        }

        system_table.boot_services().stall(10_000);

        if current_is_main && selected < payload_count && entry_available[selected] {
            if let Some(t) = parsed_timeout {
                if t > 0 {
                    ticks += 1;
                    if ticks >= 100 {
                        ticks = 0;
                        timeout -= 1;
                        redraw = true;
                        if timeout <= 0 {
                            let chosen = &main_entries[selected];
                            if chosen.action == EntryAction::LoadKernel {
                                if selected < payload_count && entry_available[selected] {
                                    target_filename = chosen.filename;
                                    debug_log(boot_config.debug, "[BOOT] timeout selected payload\r\n");
                                    break 'menu_loop;
                                }
                                parsed_timeout = None;
                                debug_log(boot_config.debug, "[ERROR] timeout target missing; staying in menu\r\n");
                                redraw = true;
                            }
                        }
                    }
                }
            }
        }
    }

    // === Выбор разрешения ===
    let mut available_res = [(0usize, 0usize); 32];
    let mut res_count = 0;

    {
        if let Ok(gop_handle) = system_table.boot_services().get_handle_for_protocol::<GraphicsOutput>() {
            let params = OpenProtocolParams {
                handle: gop_handle,
                agent: _image_handle,
                controller: None,
            };
            if let Ok(gop) = unsafe {
                system_table.boot_services().open_protocol::<GraphicsOutput>(params, OpenProtocolAttributes::GetProtocol)
            } {
                for mode in gop.modes(system_table.boot_services()) {
                    let info = mode.info();
                    let (w, h) = info.resolution();
                    if w <= 1920 && h <= 1080 && w > 0 && h > 0 {
                        let mut dup = false;
                        for i in 0..res_count {
                            if available_res[i] == (w, h) {
                                dup = true;
                                break;
                            }
                        }
                        if !dup && res_count < 32 {
                            available_res[res_count] = (w, h);
                            res_count += 1;
                        }
                    }
                }
            }
        }
    }

    for i in 0..res_count {
        for j in (i + 1)..res_count {
            if available_res[j].0 * available_res[j].1 < available_res[i].0 * available_res[i].1 {
                available_res.swap(i, j);
            }
        }
    }

    let mut res_selected = 0usize;
    if let Some((cfg_w, cfg_h)) = parsed_res {
        for i in 0..res_count {
            if available_res[i].0 == cfg_w && available_res[i].1 == cfg_h {
                res_selected = i + 1;
                break;
            }
        }
    }

    if res_selected == 0 && !boot_config.safe_mode && !boot_config.screen_auto {
        redraw = true;
        let _ = system_table.stdin().reset(false);

        loop {
        if redraw {
            let _ = system_table.stdout().clear();
            let _ = writeln!(system_table.stdout(), "========================================");
            let _ = writeln!(system_table.stdout(), "       Select Screen Resolution");
            let _ = writeln!(system_table.stdout(), "========================================");
            let _ = writeln!(system_table.stdout(), "");

            if res_selected == 0 {
                let _ = writeln!(system_table.stdout(), "   -> [ AUTO (Default) ]");
            } else {
                let _ = writeln!(system_table.stdout(), "      [ AUTO (Default) ]");
            }

            for i in 0..res_count {
                let (w, h) = available_res[i];
                if res_selected == i + 1 {
                    let _ = writeln!(system_table.stdout(), "   -> [ {}x{} ]", w, h);
                } else {
                    let _ = writeln!(system_table.stdout(), "      [ {}x{} ]", w, h);
                }
            }

            let _ = writeln!(system_table.stdout(), "\n Use UP/DOWN to select, ENTER to continue.");
            redraw = false;
        }

        if let Ok(Some(key)) = system_table.stdin().read_key() {
            match key {
                Key::Special(ScanCode::UP) => {
                    if res_selected > 0 {
                        res_selected -= 1;
                        redraw = true;
                    }
                }
                Key::Special(ScanCode::DOWN) => {
                    if res_selected < res_count {
                        res_selected += 1;
                        redraw = true;
                    }
                }
                Key::Printable(c) => {
                    let code = u16::from(c);
                    if code == 13 || code == 10 {
                        break;
                    }
                }
                _ => {}
            }
        }
            system_table.boot_services().stall(10_000);
        }
    }

    let final_res = if res_selected == 0 {
        None
    } else {
        Some(available_res[res_selected - 1])
    };

    if let Some((target_w, target_h)) = final_res {
        debug_log(boot_config.debug, "[VIDEO] applying configured GOP mode\r\n");
        if let Ok(gop_handle) = system_table.boot_services().get_handle_for_protocol::<GraphicsOutput>() {
            let params = OpenProtocolParams {
                handle: gop_handle,
                agent: _image_handle,
                controller: None,
            };
            if let Ok(mut gop) = unsafe {
                system_table.boot_services().open_protocol::<GraphicsOutput>(params, OpenProtocolAttributes::GetProtocol)
            } {
                for mode in gop.modes(system_table.boot_services()) {
                    let info = mode.info();
                    let (w, h) = info.resolution();
                    if w == target_w && h == target_h {
                        let _ = gop.set_mode(&mode);
                        break;
                    }
                }
            }
        }
    }

    let _ = system_table.stdout().clear();
    let _ = writeln!(system_table.stdout(), "Loading {}...", target_filename);
    debug_log(boot_config.debug, "[BOOT] reading payload\r\n");

    let read_result = {
        let bt = system_table.boot_services();
        read_kernel_elf(bt, target_filename)
    };

    let (kernel_buffer_addr, kernel_size) = match read_result {
        Ok((addr, sz)) => (addr, sz),
        Err(e) => {
            boot_attempt_failed = true;
            debug_log(boot_config.debug, "[ERROR] payload read failed\r\n");
            show_load_error(&mut system_table, target_filename, e);
            continue 'boot_attempt;
        }
    };

    let _ = writeln!(system_table.stdout(), "[WnkaUBTM] Loading {} ({} bytes)", target_filename, kernel_size);
    let _ = writeln!(system_table.stdout(), "-> Parsing ELF64 header...");

    if kernel_size < core::mem::size_of::<Header>() {
        boot_attempt_failed = true;
        let pages = (kernel_size + 4095) / 4096;
        let _ = unsafe { system_table.boot_services().free_pages(kernel_buffer_addr, pages) };
        debug_log(boot_config.debug, "[ERROR] ELF header is truncated\r\n");
        show_load_error(&mut system_table, target_filename, "File is too small to contain an ELF64 header.");
        continue 'boot_attempt;
    }

    let header = unsafe { &*(kernel_buffer_addr as *const Header) };
    if let Err(err) = validate_elf_header(kernel_buffer_addr, kernel_size, header) {
        boot_attempt_failed = true;
        let pages = (kernel_size + 4095) / 4096;
        let _ = unsafe { system_table.boot_services().free_pages(kernel_buffer_addr, pages) };
        debug_log(boot_config.debug, "[ERROR] ELF validation failed\r\n");
        show_load_error(&mut system_table, target_filename, err);
        continue 'boot_attempt;
    }

    let entry_point_addr = header.e_entry;
    let _ = writeln!(system_table.stdout(), "[WnkaUBTM] Entry Point: 0x{:x}", entry_point_addr);
    debug_log(boot_config.debug, "[ELF] header validated\r\n");

    let phdr_offset = header.e_phoff as usize;
    let phdr_size = header.e_phentsize as usize;
    let phdr_count = header.e_phnum as usize;

    break 'boot_attempt (kernel_buffer_addr, entry_point_addr, phdr_offset, phdr_size, phdr_count);
    };

    for i in 0..phdr_count {
        let phdr_ptr = (kernel_buffer_addr as usize + phdr_offset + i * phdr_size) as *const ProgramHeader;
        let phdr = unsafe { core::ptr::read_unaligned(phdr_ptr) };
        if phdr.p_type != PT_LOAD {
            continue;
        }

        if phdr.p_paddr % 4096 != 0 {
            halt_with_error(&mut system_table, "ELF load segment address is not page-aligned");
        }

        let seg_pages = (phdr.p_memsz as usize + 4095) / 4096;
        let phys_addr = phdr.p_paddr;
        if let Err(_) = system_table.boot_services().allocate_pages(AllocateType::Address(phys_addr), MemoryType::LOADER_DATA, seg_pages) {
            halt_with_error(&mut system_table, "Failed to allocate pages for ELF load segment");
        }

        unsafe {
            let src = (kernel_buffer_addr as usize + phdr.p_offset as usize) as *const u8;
            let dest = phys_addr as *mut u8;
            core::ptr::copy_nonoverlapping(src, dest, phdr.p_filesz as usize);
            if phdr.p_memsz > phdr.p_filesz {
                core::ptr::write_bytes(
                    (phys_addr + phdr.p_filesz) as *mut u8,
                    0,
                    (phdr.p_memsz - phdr.p_filesz) as usize,
                );
            }
        }
    }

    let _ = writeln!(system_table.stdout(), "-> Searching for ACPI tables...");

    if acpi_kind == 2 {
        let _ = writeln!(system_table.stdout(), "   [+] Found ACPI 2.0 RSDP at 0x{:x}", rsdp_addr);
    } else if acpi_kind == 1 {
        let _ = writeln!(system_table.stdout(), "   [+] Found ACPI 1.0 RSDP at 0x{:x}", rsdp_addr);
    }

    let _ = writeln!(system_table.stdout(), "-> ALL PREPARATIONS COMPLETE.");
    let _ = writeln!(system_table.stdout(), "-> GRABBING GOP (CONSOLE TEXT WILL FREEZE NOW!)...");

    let mut gop_success = false;

    if let Ok(gop_handle) = system_table.boot_services().get_handle_for_protocol::<GraphicsOutput>() {
        let params = OpenProtocolParams {
            handle: gop_handle,
            agent: _image_handle,
            controller: None,
        };
        if let Ok(mut gop) = unsafe {
            system_table.boot_services().open_protocol::<GraphicsOutput>(params, OpenProtocolAttributes::GetProtocol)
        } {
            let mode_info = gop.current_mode_info();
            let (width, height) = mode_info.resolution();

            fb_info.base_address = gop.frame_buffer().as_mut_ptr() as u64;
            fb_info.width = width as u64;
            fb_info.height = height as u64;
            fb_info.pixels_per_scan_line = mode_info.stride() as u64;

            unsafe {
                core::ptr::write_bytes(fb_info.base_address as *mut u8, 0, gop.frame_buffer().size());
            }
            gop_success = true;
        }
    }

    if !gop_success {
        halt_with_error(&mut system_table, "Graphics Output Protocol not found\n        Cannot initialize framebuffer.");
    }

    let mmap_sizes = system_table.boot_services().memory_map_size();
    let desc_size = mmap_sizes.entry_size;
    let mmap_buf_size = mmap_sizes.map_size + (8 * desc_size);
    let pages = (mmap_buf_size + 4095) / 4096;

    let mmap_ptr = match system_table.boot_services().allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages) {
        Ok(addr) => addr as *mut u8,
        Err(_) => core::ptr::null_mut(),
    };

    if mmap_ptr.is_null() {
        halt_with_error(&mut system_table, "Failed to allocate memory for UEFI Memory Map");
    }

    let mmap_slice = unsafe { core::slice::from_raw_parts_mut(mmap_ptr, mmap_buf_size) };
    let mut total_bytes = 0;
    let mut handoff_desc_size = desc_size;
    let mut mmap_success = false;

    if let Ok(mmap) = system_table.boot_services().memory_map(mmap_slice) {
        total_bytes = mmap.entries().count() * desc_size;
        mmap_success = true;
    }

    if !mmap_success {
        halt_with_error(&mut system_table, "Failed to retrieve UEFI Memory Map");
    }

    let boot_services_ptr = if boot_config.no_exit_boot_services {
        system_table.boot_services() as *const _ as *const core::ffi::c_void
    } else {
        core::ptr::null()
    };

    if !boot_config.no_exit_boot_services {
        let (_runtime_table, final_memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);
        handoff_desc_size = core::mem::size_of::<MemoryDescriptor>();
        let final_entries = final_memory_map.entries();
        total_bytes = final_entries.len() * handoff_desc_size;
        debug_log(boot_config.debug, "[MEMORY] using final post-exit memory map\r\n");

        if total_bytes > mmap_buf_size {
            loop {
                unsafe { core::arch::asm!("hlt"); }
            }
        }

        for (index, descriptor) in final_entries.enumerate() {
            unsafe {
                mmap_ptr
                    .cast::<MemoryDescriptor>()
                    .add(index)
                    .write(*descriptor);
            }
        }
    }

    let boot_info = BootInfo {
        framebuffer: fb_info,
        memory_map: mmap_ptr as *const u8,
        memory_map_size: total_bytes,
        descriptor_size: handoff_desc_size,
        rsdp: rsdp_addr,
        boot_services: boot_services_ptr,
    };

    let entry: KernelEntryPoint = unsafe { core::mem::transmute(entry_point_addr) };
    entry(&boot_info);
}

fn read_kernel_elf(bt: &uefi::table::boot::BootServices, filename: &str) -> Result<(u64, usize), &'static str> {
    let mut path_u16 = [0u16; 64];
    let len = to_utf16(filename, &mut path_u16);
    let target_cstr = match CStr16::from_u16_with_nul(&path_u16[..=len]) {
        Ok(c) => c,
        Err(_) => return Err("Invalid filename string"),
    };

    let fs_handles = bt
        .locate_handle_buffer(SearchType::ByProtocol(&SimpleFileSystem::GUID))
        .map_err(|_| "No FS handles")?;

    let mut found_file = None;
    let mut found_size = 0usize;

    for &handle in fs_handles.iter() {
        let mut fs = match bt.open_protocol_exclusive::<SimpleFileSystem>(handle) {
            Ok(fs) => fs,
            Err(_) => continue,
        };
        let mut root_dir = match fs.open_volume() {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        if let Ok(file) = root_dir.open(target_cstr, FileMode::Read, FileAttribute::empty()) {
            if let Ok(FileType::Regular(mut reg)) = file.into_type() {
                let mut info_buffer = [0u8; 512];
                if let Ok(info) = reg.get_info::<FileInfo>(&mut info_buffer) {
                    let sz = info.file_size() as usize;
                    if sz > 0 && sz <= MAX_KERNEL_SIZE {
                        found_file = Some(reg);
                        found_size = sz;
                        break;
                    }
                }
            }
        }
    }

    let mut reg = match found_file {
        Some(f) => f,
        None => return Err("Target file not found"),
    };

    let kernel_size = found_size;
    let pages = (kernel_size + 4095) / 4096;
    let kernel_buffer_addr = bt
        .allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, pages)
        .map_err(|_| "Failed to allocate memory")?;

    let buf = unsafe { core::slice::from_raw_parts_mut(kernel_buffer_addr as *mut u8, kernel_size) };
    let mut total = 0usize;
    let mut zero_reads = 0;

    while total < kernel_size {
        let remaining = kernel_size - total;
        let chunk = remaining.min(READ_CHUNK);

        match reg.read(&mut buf[total..total + chunk]) {
            Ok(0) => {
                zero_reads += 1;
                if zero_reads > 3 { break; }
                bt.stall(50_000);
            }
            Ok(n) => {
                total += n;
                zero_reads = 0;
            }
            Err(_) => {
                zero_reads += 1;
                if zero_reads > 3 { break; }
                bt.stall(50_000);
            }
        }
    }

    if total != kernel_size {
        let _ = unsafe { bt.free_pages(kernel_buffer_addr, pages) };
        return Err("Short read while loading payload");
    }

    Ok((kernel_buffer_addr, total))
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

#[no_mangle]
pub extern "C" fn wcslen(mut s: *const u16) -> usize {
    let mut len = 0;
    unsafe {
        while *s != 0 {
            len += 1;
            s = s.add(1);
        }
    }
    len
}