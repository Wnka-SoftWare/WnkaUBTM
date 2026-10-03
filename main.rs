#![no_std]
#![no_main]

// В НАЧАЛЕ ФАЙЛА:
extern crate alloc;

use uefi::allocator::Allocator;

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

use uefi::CStr16;
use core::panic::PanicInfo;
use core::fmt::Write;
use goblin::elf::header::header64::Header;
use goblin::elf::program_header::program_header64::ProgramHeader;
use goblin::elf::program_header::PT_LOAD;
use uefi::cstr16;
use uefi::prelude::*;
use uefi::Identify;
use uefi::proto::console::gop::GraphicsOutput;
use uefi::proto::console::text::{Key, ScanCode};
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, FileType};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::table::boot::{AllocateType, MemoryType, SearchType, OpenProtocolParams, OpenProtocolAttributes};
use uefi::table::cfg::{ACPI_GUID, ACPI2_GUID};



// ====================================================================
// КОНСТАНТЫ
// ====================================================================

const MAX_KERNEL_SIZE: usize = 16 * 1024 * 1024; // 16 МБ
const READ_CHUNK: usize = 65536;                  // 64 КБ

// ====================================================================
// СТРУКТУРЫ ДЛЯ ЯДРА
// ====================================================================

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
}

type KernelEntryPoint = extern "sysv64" fn(boot_info: *const BootInfo) -> !;

#[derive(Copy, Clone)]
struct BootEntry<'a> {
    name: &'a str,
    filename: &'a str,
    is_autorun: bool,
}

// Утилита для конвертации строк в UTF-16 для путей UEFI
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

// ====================================================================
// ГЛАВНАЯ ФУНКЦИЯ
// ====================================================================

#[entry]
fn main(_image_handle: Handle, mut system_table: SystemTable<Boot>) -> Status {
    if uefi::helpers::init(&mut system_table).is_err() {
        let _ = writeln!(system_table.stdout(), "Failed to initialize utilities");
        loop { system_table.boot_services().stall(1_000_000); }
    }
    
    let _ = system_table.stdout().clear();
    let _ = system_table
        .boot_services()
        .set_watchdog_timer(0, 0x10000, None);

    let _ = writeln!(system_table.stdout(), "========================================");
    let _ = writeln!(
        system_table.stdout(),
        "WnkaUBTM 0.0.1"
    );

    let mut fb_info = FramebufferInfo {
        base_address: 0,
        width: 0,
        height: 0,
        pixels_per_scan_line: 0,
    };
    let entry_point_addr: u64;
    let mut rsdp_addr: u64 = 0;

    // ====================================================================
    // ПРОХОД 1: СКАНИРУЕМ ТОМА, ИЩЕМ BOOT.CFG И БИНАРНИКИ
    // ====================================================================
    let mut has_unknown_os = false;
    let mut has_kernel = false;
    let mut has_pe = false;
    let mut cfg_buffer = [0u8; 2048];
    let mut cfg_len = 0;

    {
        let _ = writeln!(
            system_table.stdout(),
            "-> Scanning for File Systems (Disks)..."
        );

        let volume_count = {
            let bt = system_table.boot_services();
            bt.locate_handle_buffer(SearchType::ByProtocol(&SimpleFileSystem::GUID))
                .map(|h| h.len())
                .unwrap_or(0)
        };

            for idx in 0..volume_count {
        let (u, k, p) = {
        let bt = system_table.boot_services();
        let fs_handles = match bt.locate_handle_buffer(
            SearchType::ByProtocol(&SimpleFileSystem::GUID)
        ) { Ok(h) => h, Err(_) => break, };
        let handle = fs_handles[idx];
        let mut result = (false, false, false);
        if let Ok(mut fs) =
            bt.open_protocol_exclusive::<SimpleFileSystem>(handle)
        {
            if let Ok(mut root_dir) = fs.open_volume() {

                // Ищем конфиг
                if let Ok(file) = root_dir.open(
                    cstr16!("boot.cfg"),
                    FileMode::Read,
                    FileAttribute::empty()
                ) {
                    if let Ok(FileType::Regular(mut reg)) = file.into_type() {
                        cfg_len = reg.read(&mut cfg_buffer).unwrap_or(0);
                    }
                }
                let u = root_dir.open(cstr16!("kernel.elf"),FileMode::Read,FileAttribute::empty()).is_ok();
                let k = root_dir.open(cstr16!("wok.elf"),FileMode::Read,FileAttribute::empty()).is_ok();
                let p = root_dir.open(cstr16!("wnka_pe.elf"),FileMode::Read,FileAttribute::empty()).is_ok();
                result = (u, k, p);
            }
        }

        result
    };
    if u { has_unknown_os = true; }
    if k { has_kernel = true; }
    if p { has_pe = true; }
    }
}

    // ====================================================================
    // ПАРСИНГ BOOT.CFG
    // ====================================================================
    let mut entries: [BootEntry; 10] = [BootEntry { name: "", filename: "", is_autorun: false }; 10];
    let mut entry_count = 0;
    
    let mut parsed_timeout: Option<isize> = None; 
    let mut parsed_res: Option<(usize, usize)> = None;

    if cfg_len > 0 {
        if let Ok(cfg_str) = core::str::from_utf8(&cfg_buffer[..cfg_len]) {
            for line in cfg_str.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') { continue; }

                // Парсим параметр time
                if line.starts_with("time") {
                    let mut parts = line.splitn(2, '=');
                    if let (Some(_), Some(val)) = (parts.next(), parts.next()) {
                        let v = val.trim();
                        if v == "none" || v == "OFF" || v == "Off" {
                            parsed_timeout = None;
                        } else {
                            let mut num_bytes = 0;
                            for b in v.bytes() {
                                if b >= b'0' && b <= b'9' { num_bytes += 1; } else { break; }
                            }
                            if num_bytes > 0 {
                                if let Ok(secs) = core::str::from_utf8(&v.as_bytes()[..num_bytes]).unwrap_or("").parse::<isize>() {
                                    parsed_timeout = Some(secs);
                                }
                            }
                        }
                    }
                    continue;
                }

                // Парсим разрешение экрана
                if line.starts_with("screen") {
                    let mut parts = line.splitn(2, '=');
                    if let (Some(_), Some(val)) = (parts.next(), parts.next()) {
                        let v = val.trim();
                        let mut dim_parts = v.split('x');
                        if let (Some(w_str), Some(h_str)) = (dim_parts.next(), dim_parts.next()) {
                            if let (Ok(w), Ok(h)) = (w_str.trim().parse::<usize>(), h_str.trim().parse::<usize>()) {
                                parsed_res = Some((w, h));
                            }
                        }
                    }
                    continue;
                }

                // Парсим пункты загрузки
                let mut parts = line.splitn(2, '=');
                if let (Some(name_part), Some(file_part)) = (parts.next(), parts.next()) {
                    if entry_count < 10 {
                        let raw_name = name_part.trim();
                        let mut clean_file = file_part.trim();
                        let mut clean_name = raw_name;
                        let mut is_auto = false;

                        // Ищем маркер автозапуска в названии или в файле
                        if let Some(idx) = raw_name.find("<- autorun") {
                            is_auto = true;
                            clean_name = raw_name[..idx].trim();
                        } else if let Some(idx) = clean_file.find("<- autorun") {
                            is_auto = true;
                            clean_file = clean_file[..idx].trim();
                        }

                        entries[entry_count] = BootEntry { 
                            name: clean_name, 
                            filename: clean_file,
                            is_autorun: is_auto 
                        };
                        entry_count += 1;
                    }
                }
            }
        }
    }

    // Если конфига нет или он пуст - фоллбэк на классический вид
    if entry_count == 0 {
        if has_unknown_os {
            entries[entry_count] = BootEntry { name: "Untitled OS", filename: "kernel.elf", is_autorun: false };
            entry_count += 1;
        }
        if has_kernel {
            entries[entry_count] = BootEntry { name: "WnkaU4X", filename: "wok.elf", is_autorun: false };
            entry_count += 1;
        }
        if has_pe {
            entries[entry_count] = BootEntry { name: "WnkaPE Installer (wnka_pe.elf)", filename: "wnka_pe.elf", is_autorun: false };
            entry_count += 1;
        }
    }

    if entry_count == 0 {
        let _ = writeln!(
            system_table.stdout(),
            "[WnkaOS Bootloader] Boot failure: no payloads found!"
        );
        loop { system_table.boot_services().stall(1_000_000); }
    }

    // ====================================================================
    // МЕНЮ 1: ВЫБОР ОС
    // ====================================================================
    let mut selected = 0usize;
    for i in 0..entry_count {
        if entries[i].is_autorun {
            selected = i;
            break;
        }
    }

    let mut redraw = true;
    let mut timeout = parsed_timeout.unwrap_or(-1);
    let mut ticks = 0;
    let _ = system_table.stdin().reset(false);

    loop {
        if redraw {
            let _ = system_table.stdout().clear();
            let _ = writeln!(system_table.stdout(), "========================================");
            let _ = writeln!(system_table.stdout(), "           Wnka Boot Manager");
            let _ = writeln!(system_table.stdout(), "========================================");
            let _ = writeln!(system_table.stdout(), "");

            for i in 0..entry_count {
                if i == selected {
                    let _ = writeln!(system_table.stdout(), "   -> [ {} ]", entries[i].name);
                } else {
                    let _ = writeln!(system_table.stdout(), "      [ {} ]", entries[i].name);
                }
            }

            let _ = writeln!(system_table.stdout(), "");
            if parsed_timeout.is_some() && timeout > 0 {
                let _ = writeln!(system_table.stdout(), " Booting automatically in {} seconds...", timeout);
                let _ = writeln!(system_table.stdout(), " (Press any arrow key to stop timer)");
            } else {
                let _ = writeln!(system_table.stdout(), " Use UP/DOWN to select, ENTER to boot.");
            }
            redraw = false;
        }

        let mut key_pressed = false;
        if let Ok(Some(key)) = system_table.stdin().read_key() {
            key_pressed = true;
            parsed_timeout = None; // Отключаем таймер
            match key {
                Key::Special(ScanCode::UP) => {
                    if selected > 0 { selected -= 1; }
                }
                Key::Special(ScanCode::DOWN) => {
                    if selected < entry_count - 1 { selected += 1; }
                }
                Key::Printable(c) => {
                    let code = u16::from(c);
                    if code == 13 || code == 10 { break; }
                }
                _ => {}
            }
        }

        if key_pressed {
            redraw = true;
            continue;
        }

        system_table.boot_services().stall(10_000);
        if let Some(t) = parsed_timeout {
            if t > 0 {
                ticks += 1;
                if ticks >= 100 {
                    ticks = 0;
                    timeout -= 1;
                    redraw = true;
                    if timeout <= 0 { break; }
                }
            }
        }
    }

    let target_filename = entries[selected].filename;

    // ====================================================================
    // МЕНЮ 2: ВЫБОР РАЗРЕШЕНИЯ ЭКРАНА
    // ====================================================================
    let mut available_res = [(0usize, 0usize); 32];
    let mut res_count = 0;

    {
        if let Ok(gop_handle) = system_table.boot_services().get_handle_for_protocol::<GraphicsOutput>() {
            let params = OpenProtocolParams {
                handle: gop_handle,
                agent: _image_handle,
                controller: None,
            };
            if let Ok(gop) = unsafe { system_table.boot_services().open_protocol::<GraphicsOutput>(params, OpenProtocolAttributes::GetProtocol) } {
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

    // Сортируем по площади
    for i in 0..res_count {
        for j in (i + 1)..res_count {
            if available_res[j].0 * available_res[j].1 < available_res[i].0 * available_res[i].1 {
                available_res.swap(i, j);
            }
        }
    }

    let mut res_selected = 0usize; // 0 = AUTO

    // Если в boot.cfg было указано разрешение, ищем его в списке
    if let Some((cfg_w, cfg_h)) = parsed_res {
        for i in 0..res_count {
            if available_res[i].0 == cfg_w && available_res[i].1 == cfg_h {
                res_selected = i + 1; // +1 потому что 0 это AUTO
                break;
            }
        }
    }

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

            let _ = writeln!(
                system_table.stdout(),
                "\n Use UP/DOWN to select, ENTER to continue."
            );
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

    let final_res = if res_selected == 0 {
        None
    } else {
        Some(available_res[res_selected - 1])
    };

    // ====================================================================
    // ПРИМЕНЯЕМ РАЗРЕШЕНИЕ ЭКРАНА ДО ТОГО КАК НАЧНЕМ ПЕЧАТАТЬ ЛОГИ!
    // ====================================================================
    if let Some((target_w, target_h)) = final_res {
        if let Ok(gop_handle) = system_table.boot_services().get_handle_for_protocol::<GraphicsOutput>() {
            let params = OpenProtocolParams {
                handle: gop_handle,
                agent: _image_handle,
                controller: None,
            };
            if let Ok(mut gop) = unsafe { system_table.boot_services().open_protocol::<GraphicsOutput>(params, OpenProtocolAttributes::GetProtocol) } {
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

    // ====================================================================
    // ЧИТАЕМ ELF
    // ====================================================================
    let _ = system_table.stdout().clear(); // Сбросим позицию курсора на новом разрешении
    let _ = writeln!(system_table.stdout(), "Loading {}...", target_filename);

    let read_result = {
        let bt = system_table.boot_services();
        read_kernel_elf(bt, target_filename)
    };

    let (kernel_buffer_addr, kernel_size) = match read_result {
        Ok((addr, sz)) => (addr, sz),
        Err(e) => {
            let _ = writeln!(
                system_table.stdout(),
                "[WnkaOS Bootloader] {} for '{}'",
                e, target_filename
            );
            loop {
                system_table.boot_services().stall(1_000_000);
            }
        }
    };

    let _ = writeln!(
        system_table.stdout(),
        "[WnkaOS Bootloader] Loading {} ({} bytes)",
        target_filename,
        kernel_size
    );

    // ====================================================================
    // ELF PARSING
    // ====================================================================
    let _ = writeln!(system_table.stdout(), "-> Parsing ELF64 header...");
    let header = unsafe { &*(kernel_buffer_addr as *const Header) };
    if &header.e_ident[0..4] != b"\x7fELF" {
        let _ = writeln!(system_table.stdout(), "[WnkaOS Bootloader] Invalid ELF header!");
        loop { system_table.boot_services().stall(1_000_000); }
    }

    entry_point_addr = header.e_entry;
    let _ = writeln!(
        system_table.stdout(),
        "[WnkaOS Bootloader] Entry Point: 0x{:x}",
        entry_point_addr
    );

    let phdr_offset = header.e_phoff as usize;
    let phdr_size = header.e_phentsize as usize;
    let phdr_count = header.e_phnum as usize;

    for i in 0..phdr_count {
        let phdr_ptr =
            (kernel_buffer_addr as usize + phdr_offset + i * phdr_size) as *const ProgramHeader;
        let phdr = unsafe { &*phdr_ptr };
        if phdr.p_type == PT_LOAD {
            let seg_pages = (phdr.p_memsz as usize + 4095) / 4096;
            let phys_addr = phdr.p_paddr;
            {
                let bt = system_table.boot_services();
                let _ = bt.allocate_pages(
                    AllocateType::Address(phys_addr),
                    MemoryType::LOADER_DATA,
                    seg_pages,
                );
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
    }

    // ====================================================================
    // ACPI
    // ====================================================================
    let _ = writeln!(system_table.stdout(), "-> Searching for ACPI tables...");

    let mut acpi_kind: u8 = 0;
    for entry in system_table.config_table() {
        if entry.guid == ACPI2_GUID {
            rsdp_addr = entry.address as u64;
            acpi_kind = 2;
            break;
        } else if entry.guid == ACPI_GUID {
            rsdp_addr = entry.address as u64;
            acpi_kind = 1;
        }
    }

    if acpi_kind == 2 {
        let _ = writeln!(
            system_table.stdout(),
            "   [+] Found ACPI 2.0 RSDP at 0x{:x}",
            rsdp_addr
        );
    } else if acpi_kind == 1 {
        let _ = writeln!(
            system_table.stdout(),
            "   [+] Found ACPI 1.0 RSDP at 0x{:x}",
            rsdp_addr
        );
    }

    let _ = writeln!(system_table.stdout(), "-> ALL PREPARATIONS COMPLETE.");
    let _ = writeln!(
        system_table.stdout(),
        "-> GRABBING GOP (CONSOLE TEXT WILL FREEZE NOW!)..."
    );

    // ====================================================================
    // GOP + MEMORY MAP 
    // ====================================================================
    let mut gop_success = false;

    if let Ok(gop_handle) = system_table.boot_services().get_handle_for_protocol::<GraphicsOutput>() {
        let params = OpenProtocolParams {
            handle: gop_handle,
            agent: _image_handle,
            controller: None,
        };
        if let Ok(mut gop) = unsafe { system_table.boot_services().open_protocol::<GraphicsOutput>(params, OpenProtocolAttributes::GetProtocol) } {
        
        let mode_info = gop.current_mode_info();
        let (width, height) = mode_info.resolution();

        fb_info.base_address = gop.frame_buffer().as_mut_ptr() as u64;
        fb_info.width = width as u64;
        fb_info.height = height as u64;
        
        // ФИКС 1: Жестко привязываем шаг к ширине экрана, обходя баг QEMU
        fb_info.pixels_per_scan_line = width as u64;
        
        // ФИКС 2: Зачищаем фреймбуфер от текста UEFI нулями (черным цветом)
        unsafe {
            core::ptr::write_bytes(fb_info.base_address as *mut u8, 0, gop.frame_buffer().size());
        }

        gop_success = true;
    }
    }

    if !gop_success {
        let _ = writeln!(system_table.stdout(), "[WnkaOS Bootloader] NO GOP FOUND");
        loop { system_table.boot_services().stall(1_000_000); }
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
        let _ = writeln!(system_table.stdout(), "[WnkaOS Bootloader] MEMORY MAP ALLOCATION FAILED");
        loop { system_table.boot_services().stall(1_000_000); }
    }

    let mmap_slice = unsafe { core::slice::from_raw_parts_mut(mmap_ptr, mmap_buf_size) };
    
    let mut total_bytes = 0;
    let mut mmap_success = false;

    if let Ok(mmap) = system_table.boot_services().memory_map(mmap_slice) {
        total_bytes = mmap.entries().count() * desc_size;
        mmap_success = true;
    }

    if !mmap_success {
        let _ = writeln!(system_table.stdout(), "[WnkaOS Bootloader] MEMORY MAP GET FAILED");
        loop { system_table.boot_services().stall(1_000_000); }
    }

    // ====================================================================
    // ФИНАЛ
    // ====================================================================
    let _runtime_table = system_table.exit_boot_services(MemoryType::LOADER_DATA);

    let boot_info = BootInfo {
        framebuffer: fb_info,
        memory_map: mmap_ptr as *const u8,
        memory_map_size: total_bytes,
        descriptor_size: desc_size,
        rsdp: rsdp_addr,
    };

    let entry: KernelEntryPoint = unsafe { core::mem::transmute(entry_point_addr) };
    entry(&boot_info);
}

// ====================================================================
// ЧТЕНИЕ ELF
// ====================================================================

fn read_kernel_elf(
bt: &uefi::table::boot::BootServices,
    filename: &str,
) -> Result<(u64, usize), &'static str> {
    
    // Динамически конвертируем переданное имя файла в UTF-16
    let mut path_u16 = [0u16; 64];
    let len = to_utf16(filename, &mut path_u16); // Сохраняем длину!
    
    // Передаем срез строго до нуль-терминатора включительно (len + 1 элемент)
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

    let buf = unsafe {
        core::slice::from_raw_parts_mut(kernel_buffer_addr as *mut u8, kernel_size)
    };

    // === ФИКС: устойчивое чтение с повторами ===
    let mut total = 0usize;
    let mut zero_reads = 0;
    
    while total < kernel_size {
        let remaining = kernel_size - total;
        let chunk = remaining.min(READ_CHUNK);
        
        match reg.read(&mut buf[total..total + chunk]) {
            Ok(0) => {
                zero_reads += 1;
                if zero_reads > 3 {
                    // Реально конец файла
                    break;
                }
                // Даём флешке время
                bt.stall(50_000); // 50 мс
            }
            Ok(n) => {
                total += n;
                zero_reads = 0;
            }
            Err(_) => {
                zero_reads += 1;
                if zero_reads > 3 {
                    break;
                }
                bt.stall(50_000);
            }
        }
    }

    // === ФИКС: если прочитали меньше — используем реальный размер ===
    // Это временное решение. Настоящая причина — в инсталляторе.
    if total == 0 {
        return Err("Read mismatch: 0 bytes");
    }
    
    // Возвращаем реально прочитанный размер
    Ok((kernel_buffer_addr, total))
}

// ====================================================================
// PANIC HANDLER
// ====================================================================

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}
// ====================================================================
// WCSLEN (требуется uefi-крейту на линковке)
// ====================================================================

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