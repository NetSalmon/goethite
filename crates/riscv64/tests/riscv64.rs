use goethite_config::KERNEL_OFFSET;
use goethite_riscv64::{
    InterruptGuard,
    addr::{
        AsPhysAddr, AsVirtAddr, GetPpn, GetVpn, PhysicalAddr, PhysicalPageNumber, VirtualAddr,
        VirtualPageNumber,
    },
    address_translation_fence, breakpoint, disable_interrupt, enable_interrupt, hart_id,
    instruction_fence,
    registers::{
        Registers,
        gpr::Tp,
        values::{SatpMode, SatpValue, SstatusValue},
    },
    trap::{Exception, Interrupt, Trap},
    wait_for_interrupt,
};

// ===== Sv39 地址 =====

#[test]
fn virtual_address_bit_fields() {
    let addr = VirtualAddr::from(0x0000_0000_8000_1234usize);
    assert_eq!(addr.page_offset(), 0x234);
    assert_eq!(addr.vpn0(), 1);
    assert_eq!(addr.vpn1(), 0);
    assert_eq!(addr.vpn2(), 2);
    assert_eq!(addr.vpn(), 0x80001);

    assert_eq!(addr.get_vpn(VirtualPageNumber::Vpn0), 1);
    assert_eq!(addr.get_vpn(VirtualPageNumber::Vpn1), 0);
    assert_eq!(addr.get_vpn(VirtualPageNumber::Vpn2), 2);
}

#[test]
fn physical_address_bit_fields() {
    let addr = PhysicalAddr::from(0x0000_0000_8000_1234usize);
    assert_eq!(addr.page_offset(), 0x234);
    assert_eq!(addr.ppn0(), 1);
    assert_eq!(addr.ppn1(), 0);
    assert_eq!(addr.ppn2(), 0x2); // bits 30..55
    assert_eq!(addr.ppn(), 0x80001);

    assert_eq!(addr.get_ppn(PhysicalPageNumber::Ppn0), 1);
    assert_eq!(addr.get_ppn(PhysicalPageNumber::Ppn1), 0);
    assert_eq!(addr.get_ppn(PhysicalPageNumber::Ppn2), 0x2);
}

#[test]
fn physical_virtual_conversions_use_kernel_offset() {
    let paddr = PhysicalAddr::from(0x8000_0000usize);
    let vaddr: VirtualAddr = paddr.clone().as_vaddr();
    assert_eq!(*vaddr, KERNEL_OFFSET + 0x8000_0000);
    assert_eq!(*vaddr.as_paddr(), 0x8000_0000);

    let vaddr = VirtualAddr::from(KERNEL_OFFSET + 0x1000);
    let paddr: PhysicalAddr = vaddr.clone().into();
    assert_eq!(*paddr, 0x1000);
    assert_eq!(*vaddr.as_paddr(), 0x1000);

    let as_u64: u64 = PhysicalAddr::from(0x5000usize).into();
    assert_eq!(as_u64, 0x5000);
}

#[test]
fn address_pointer_arithmetic() {
    let addr = PhysicalAddr::from(0x1000usize);
    let next = addr.add::<u32>(4);
    assert_eq!(*next, 0x1010);
    let next = next.add::<u64>(2);
    assert_eq!(*next, 0x1020);

    assert_eq!(addr.as_ptr::<u8>(), 0x1000usize as *const u8);
    assert_eq!(addr.as_mut_ptr::<u8>(), 0x1000usize as *mut u8);

    let vaddr = VirtualAddr::from(0x2000usize);
    assert_eq!(*vaddr.add::<u8>(3), 0x2003);
    assert_eq!(vaddr.as_ptr::<u8>(), 0x2000usize as *const u8);
    assert_eq!(vaddr.as_mut_ptr::<u8>(), 0x2000usize as *mut u8);
}

#[test]
fn as_paddr_and_as_vaddr_for_references() {
    let value = 0xdead_beefu32;
    let stack_address = &value as *const u32 as usize;
    let reference = &value;
    let address = AsPhysAddr::as_paddr(&reference);
    assert_eq!(*address, stack_address);
    let address = AsVirtAddr::as_vaddr(&reference);
    assert_eq!(*address, stack_address);

    let mut value = 0u32;
    let mutable_reference = &mut value;
    let address = AsPhysAddr::as_paddr(&mutable_reference);
    assert_eq!(*address, mutable_reference as *mut u32 as usize);
    let address = AsVirtAddr::as_vaddr(&mutable_reference);
    assert_eq!(*address, mutable_reference as *mut u32 as usize);
}

// ===== 陷阱 =====

#[test]
fn trap_parses_exceptions() {
    assert!(matches!(
        Trap::from(0i64),
        Trap::Exception(Exception::INSTRUCTION_ADDRESS_MISALIGNED)
    ));
    assert!(matches!(
        Trap::from(2i64),
        Trap::Exception(Exception::ILLEGAL_INSTRUCTION)
    ));
    assert!(matches!(
        Trap::from(9i64),
        Trap::Exception(Exception::S_MODE_ECALL)
    ));
    assert!(matches!(
        Trap::from(15i64),
        Trap::Exception(Exception::STORE_PAGE_FAULT)
    ));
}

#[test]
fn trap_parses_interrupts() {
    let mask = i64::MIN; // bit 63
    assert!(matches!(
        Trap::from(mask),
        Trap::Interrupt(Interrupt::U_MODE_SOFTWARE)
    ));
    assert!(matches!(
        Trap::from(mask | 1),
        Trap::Interrupt(Interrupt::S_MODE_SOFTWARE)
    ));
    assert!(matches!(
        Trap::from(mask | 5),
        Trap::Interrupt(Interrupt::SUPERVISOR_TIMER)
    ));
    assert!(matches!(
        Trap::from(mask | 9),
        Trap::Interrupt(Interrupt::SUPERVISOR_EXTERNAL)
    ));
}

#[test]
fn trap_roundtrips_through_i64() {
    let cases = [
        Trap::Exception(Exception::BREAKPOINT),
        Trap::Exception(Exception::U_MODE_ECALL),
        Trap::Interrupt(Interrupt::SUPERVISOR_TIMER),
        Trap::Interrupt(Interrupt::MACHINE_EXTERNAL),
    ];
    for trap in cases {
        let value: i64 = trap.into();
        assert_eq!(i64::from(Trap::from(value)), value);
    }

    assert_eq!(Exception::S_MODE_ECALL.0, 9);
    assert_eq!(Interrupt::SUPERVISOR_EXTERNAL.0, i64::MIN | 9);
    assert_eq!(i64::from(Exception::STORE_PAGE_FAULT), 15);

    let from_exception: Trap = Exception::ILLEGAL_INSTRUCTION.into();
    assert!(matches!(
        from_exception,
        Trap::Exception(Exception::ILLEGAL_INSTRUCTION)
    ));
    let from_interrupt: Trap = Interrupt::USER_TIMER.into();
    assert!(matches!(
        from_interrupt,
        Trap::Interrupt(Interrupt::USER_TIMER)
    ));
}

// ===== 寄存器与 CSR 值 =====

#[test]
fn register_snapshot_layout() {
    assert_eq!(size_of::<Registers>(), 256);
    let mut registers = Registers::new();
    assert_eq!(registers.a0, 0);
    assert_eq!(registers.args(), [0; 8]);

    registers.a0 = 10;
    registers.a1 = 11;
    registers.a7 = 17;
    assert_eq!(registers.args()[0], 10);
    assert_eq!(registers.args()[1], 11);
    assert_eq!(registers.args()[7], 17);
}

#[test]
fn satp_and_sstatus_values() {
    let satp = SatpValue::builder()
        .set_ppn(0x80000)
        .set_asid(3)
        .set_mode(SatpMode::SV39.0)
        .build();
    assert_eq!(satp.ppn(), 0x80000);
    assert_eq!(satp.asid(), 3);
    assert_eq!(satp.mode(), SatpMode::SV39.0);
    assert_eq!(u64::from(satp), (8u64 << 60) | (3u64 << 44) | 0x80000);

    let sstatus = SstatusValue::from(0x22u64);
    assert!(sstatus.spie());
    assert!(sstatus.sie());
    assert!(!sstatus.spp());
    assert_eq!(SatpMode::BARE.0, 0);
    assert_eq!(SatpMode::SV39.0, 8);
}

#[test]
fn host_register_primitives() {
    assert_eq!(Tp::read(), 0);
    Tp::write(7);
    assert_eq!(Tp::read(), 0, "host stub ignores writes");
    assert_eq!(hart_id(), 0);

    assert!(!disable_interrupt());
    enable_interrupt(true);
    enable_interrupt(false);
    drop(InterruptGuard::new());

    wait_for_interrupt();
    instruction_fence();
    address_translation_fence();
    breakpoint();
}
