use goethite_elf::{
    ElfClass, ElfError, ElfHeader, ElfIdent, ElfMachine, ElfType, Endianness, ProgramFlags,
    ProgramHeader, ProgramType,
};

/// 构造一个最小的 ELF64 文件头字节缓冲区。
fn elf_header_bytes() -> Vec<u8> {
    let mut bytes = vec![0u8; size_of::<ElfHeader>()];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = ElfClass::CLASS64.0; // e_ident[EI_CLASS]
    bytes[5] = Endianness::LSB.0; // e_ident[EI_DATA]
    bytes[6] = 1; // e_ident[EI_VERSION]
    bytes[7] = 3; // e_ident[EI_OSABI] = Linux
    bytes[8] = 0; // e_ident[EI_ABIVERSION]

    bytes[16..18].copy_from_slice(&u16::from(ElfType::EXEC).to_le_bytes());
    bytes[18..20].copy_from_slice(&u16::from(ElfMachine::RISCV).to_le_bytes());
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    bytes[24..32].copy_from_slice(&0x0040_1000u64.to_le_bytes()); // e_entry
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
    bytes[40..48].copy_from_slice(&0u64.to_le_bytes()); // e_shoff
    bytes[52..54].copy_from_slice(&64u16.to_le_bytes()); // e_ehsize
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
    bytes[56..58].copy_from_slice(&2u16.to_le_bytes()); // e_phnum
    bytes
}

#[test]
fn parse_and_validate_header() {
    let bytes = elf_header_bytes();
    let header = ElfHeader::try_from(&bytes[..]).expect("header should parse");

    assert!(header.ident.is_elf());
    assert_eq!(header.ident.class(), ElfClass::CLASS64);
    assert_eq!(header.ident.data(), Endianness::LSB);
    assert_eq!(header.r#type, ElfType::EXEC);
    assert_eq!(header.machine, ElfMachine::RISCV);
    assert_eq!(header.entry, 0x0040_1000);
    assert_eq!(header.program_header_offset, 64);
    assert_eq!(header.program_header_entry_size, 56);
    assert_eq!(header.program_header_num, 2);
    assert!(header.validate().is_ok());

    // 结构体布局与 ELF64 规范一致。
    assert_eq!(size_of::<ElfHeader>(), 64);
}

#[test]
fn short_buffer_is_rejected() {
    let bytes = [0u8; 8];
    assert!(matches!(
        ElfHeader::try_from(&bytes[..]),
        Err(ElfError::NotElf)
    ));
}

#[test]
fn validate_reports_reason() {
    let mut bytes = elf_header_bytes();
    bytes[0] = b'X';
    let header = ElfHeader::try_from(&bytes[..]).unwrap();
    assert!(matches!(header.validate(), Err(ElfError::NotElf)));

    let mut bytes = elf_header_bytes();
    bytes[5] = Endianness::MSB.0;
    let header = ElfHeader::try_from(&bytes[..]).unwrap();
    assert!(matches!(header.validate(), Err(ElfError::NotLsb)));

    let mut bytes = elf_header_bytes();
    bytes[4] = ElfClass::CLASS32.0;
    let header = ElfHeader::try_from(&bytes[..]).unwrap();
    assert!(matches!(header.validate(), Err(ElfError::Not64Bit)));

    let mut bytes = elf_header_bytes();
    bytes[18..20].copy_from_slice(&u16::from(ElfMachine::X86_64).to_le_bytes());
    let header = ElfHeader::try_from(&bytes[..]).unwrap();
    assert!(matches!(header.validate(), Err(ElfError::NotRiscv)));
}

#[test]
fn ident_helpers_and_numeric_conversions() {
    let mut ident = ElfIdent([0u8; 16]);
    assert!(!ident.is_elf());
    ident.0[0..4].copy_from_slice(b"\x7fELF");
    assert!(ident.is_elf());

    let mut ident = ElfIdent([0u8; 16]);
    ident.set_class(ElfClass::CLASS64);
    ident.set_data(Endianness::LSB);
    ident.set_os_abi(3u8.into());
    ident.set_abi_version(7);
    ident.set_n_index(15);
    assert_eq!(ident.class(), ElfClass::CLASS64);
    assert_eq!(ident.data(), Endianness::LSB);
    assert_eq!(u8::from(ident.os_abi()), 3);
    assert_eq!(ident.pad_start(), 0);
    assert_eq!(ident.n_index(), 15);

    assert_eq!(u16::from(ElfType::EXEC), 2);
    assert_eq!(ElfMachine::from(0x00F3u16), ElfMachine::RISCV);
}

#[test]
fn program_header_parse_and_flags() {
    let mut bytes = vec![0u8; size_of::<ProgramHeader>()];
    bytes[0..4].copy_from_slice(&u32::from(ProgramType::LOAD).to_le_bytes());
    bytes[4..8].copy_from_slice(&0b111u32.to_le_bytes()); // R | W | X
    bytes[8..16].copy_from_slice(&0x1000u64.to_le_bytes());
    bytes[16..24].copy_from_slice(&0x0040_0000u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&0x200u64.to_le_bytes());
    bytes[40..48].copy_from_slice(&0x300u64.to_le_bytes());
    bytes[48..56].copy_from_slice(&0x1000u64.to_le_bytes());

    let header = ProgramHeader::try_from(&bytes[..]).expect("program header should parse");
    assert_eq!(size_of::<ProgramHeader>(), 56);
    assert_eq!(header.r#type, ProgramType::LOAD);
    assert_eq!(header.offset, 0x1000);
    assert_eq!(header.virtual_addr, 0x0040_0000);
    assert_eq!(header.file_size, 0x200);
    assert_eq!(header.mem_size, 0x300);
    assert_eq!(header.align, 0x1000);

    assert!(header.flags.read());
    assert!(header.flags.write());
    assert!(header.flags.execute());
    assert_eq!(format!("{}", header.flags), "rwx");

    let mut flags = ProgramFlags::new();
    assert_eq!(format!("{flags}"), "---");
    flags.set_read(true);
    flags.set_execute(true);
    assert_eq!(format!("{flags}"), "r-x");

    let built = ProgramFlags::builder().set_write(true).build();
    assert_eq!(u32::from(built), 0b010);
    assert!(!built.read());
}

#[test]
fn short_program_header_is_rejected() {
    let bytes = [0u8; 8];
    assert!(matches!(
        ProgramHeader::try_from(&bytes[..]),
        Err(ElfError::NotElf)
    ));
}
