use goethite_riscv64_asm::{
    AmoOp, AmoWidth, CompressedInstruction, Encoded, Instruction, fence, reg,
};

#[test]
fn rv64i_encodings() {
    let cases: &[(Instruction, u32)] = &[
        (
            Instruction::Add {
                rd: reg::A0,
                rs1: reg::A0,
                rs2: reg::A1,
            },
            0x00b5_0533,
        ),
        (
            Instruction::Sub {
                rd: reg::A0,
                rs1: reg::A1,
                rs2: reg::A2,
            },
            0x40c5_8533,
        ),
        (
            Instruction::Addi {
                rd: reg::A0,
                rs1: reg::ZERO,
                imm: 1,
            },
            0x0010_0513,
        ),
        (
            Instruction::Addi {
                rd: reg::SP,
                rs1: reg::SP,
                imm: -16,
            },
            0xff01_0113,
        ),
        (
            Instruction::Lui {
                rd: reg::A0,
                imm20: 0x12345,
            },
            0x1234_5537,
        ),
        (
            Instruction::Auipc {
                rd: reg::RA,
                imm20: 0x1,
            },
            0x0000_1097,
        ),
        (
            Instruction::Jal {
                rd: reg::RA,
                offset: 8,
            },
            0x0080_00ef,
        ),
        (
            Instruction::Jalr {
                rd: reg::ZERO,
                rs1: reg::RA,
                offset: 0,
            },
            0x0000_8067,
        ),
        (
            Instruction::Ld {
                rd: reg::A0,
                rs1: reg::SP,
                offset: 8,
            },
            0x0081_3503,
        ),
        (
            Instruction::Sd {
                rs2: reg::A1,
                rs1: reg::A0,
                offset: 16,
            },
            0x00b5_3823,
        ),
        (
            Instruction::Beq {
                rs1: reg::A0,
                rs2: reg::A1,
                offset: 8,
            },
            0x00b5_0463,
        ),
        (
            Instruction::Slli {
                rd: reg::A0,
                rs1: reg::A0,
                shamt: 3,
            },
            0x0035_1513,
        ),
        (
            Instruction::Srai {
                rd: reg::A0,
                rs1: reg::A0,
                shamt: 3,
            },
            0x4035_5513,
        ),
    ];

    for (instruction, expected) in cases {
        assert_eq!(
            instruction.encode(),
            *expected,
            "unexpected encoding for {instruction:?}"
        );
    }
}

#[test]
fn rv64m_and_atomic_encodings() {
    assert_eq!(
        Instruction::Mul {
            rd: reg::A0,
            rs1: reg::A0,
            rs2: reg::A1,
        }
        .encode(),
        0x02b5_0533
    );
    assert_eq!(
        Instruction::Divu {
            rd: reg::A0,
            rs1: reg::A1,
            rs2: reg::A2,
        }
        .encode(),
        0x02c5_d533
    );
    assert_eq!(
        Instruction::Lr {
            width: AmoWidth::Double,
            rd: reg::A0,
            rs1: reg::A1,
            aq: false,
            rl: false,
        }
        .encode(),
        0x1005_b52f
    );
    assert_eq!(
        Instruction::Amo {
            op: AmoOp::Swap,
            width: AmoWidth::Word,
            rd: reg::A0,
            rs1: reg::A1,
            rs2: reg::A2,
            aq: true,
            rl: false,
        }
        .encode(),
        0x0cc5_a52f
    );
}

#[test]
fn system_and_fence_encodings() {
    assert_eq!(Instruction::Ecall.encode(), 0x0000_0073);
    assert_eq!(Instruction::Ebreak.encode(), 0x0010_0073);
    assert_eq!(Instruction::Mret.encode(), 0x3020_0073);
    assert_eq!(Instruction::Sret.encode(), 0x1020_0073);
    assert_eq!(Instruction::Wfi.encode(), 0x1050_0073);
    assert_eq!(
        Instruction::Csrrs {
            rd: reg::A0,
            rs1: reg::ZERO,
            csr: 0x300,
        }
        .encode(),
        0x3000_2573
    );
    assert_eq!(
        Instruction::Fence {
            fm: 0,
            predecessor: fence::RW,
            successor: fence::RW,
        }
        .encode(),
        0x0330_000f
    );
    assert_eq!(
        Instruction::SfenceVma {
            rs1: reg::ZERO,
            rs2: reg::ZERO,
        }
        .encode(),
        0x1200_0073
    );
}

#[test]
fn compressed_encodings() {
    let cases: &[(CompressedInstruction, u16)] = &[
        (CompressedInstruction::Nop, 0x0001),
        (CompressedInstruction::Ebreak, 0x9002),
        (
            CompressedInstruction::Addi {
                rd: reg::A0,
                imm: -1,
            },
            0x157d,
        ),
        (
            CompressedInstruction::Mv {
                rd: reg::A0,
                rs2: reg::A1,
            },
            0x852e,
        ),
        (CompressedInstruction::J { offset: 0 }, 0xa001),
    ];

    for (instruction, expected) in cases {
        assert_eq!(
            instruction.encode(),
            *expected,
            "unexpected encoding for {instruction:?}"
        );
    }

    assert_eq!(Instruction::SIZE, 4);
    assert_eq!(CompressedInstruction::SIZE, 2);
    assert_eq!(Instruction::Ecall.size(), Instruction::SIZE);
    assert_eq!(CompressedInstruction::Nop.size(), 2);
}

#[test]
fn encoded_read_and_write_roundtrip() {
    let add = Encoded::from(Instruction::Add {
        rd: reg::A0,
        rs1: reg::A0,
        rs2: reg::A1,
    });
    assert!(!add.is_compressed());
    assert_eq!(add.size(), 4);

    let mut buffer = [0u8; 4];
    assert_eq!(add.write_to(&mut buffer), Some(4));
    assert_eq!(buffer, 0x00b5_0533u32.to_le_bytes());
    // SAFETY: 缓冲区至少 4 字节可读。
    let read_back = unsafe { Encoded::read(buffer.as_ptr()) };
    assert_eq!(read_back, add);

    let compressed = Encoded::from(CompressedInstruction::Mv {
        rd: reg::A0,
        rs2: reg::A1,
    });
    assert!(compressed.is_compressed());
    assert_eq!(compressed.size(), 2);

    let mut short = [0u8; 2];
    assert_eq!(compressed.write_to(&mut short), Some(2));
    assert_eq!(short, 0x852eu16.to_le_bytes());
    // SAFETY: 缓冲区至少 2 字节可读，且低两位不是 0b11。
    let read_back = unsafe { Encoded::read(short.as_ptr()) };
    assert_eq!(read_back, compressed);

    let mut too_small = [0u8; 2];
    assert_eq!(add.write_to(&mut too_small), None);
}

#[test]
fn register_aliases() {
    assert_eq!(reg::ZERO, 0);
    assert_eq!(reg::RA, 1);
    assert_eq!(reg::SP, 2);
    assert_eq!(reg::TP, 4);
    assert_eq!(reg::S0, reg::FP);
    assert_eq!(reg::A0, 10);
    assert_eq!(reg::T6, 31);

    assert_eq!(fence::RW, fence::R | fence::W);
    assert_eq!(fence::I, 1 << 3);
    assert_eq!(fence::O, 1 << 2);
}
