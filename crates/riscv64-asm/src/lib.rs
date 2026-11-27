#![no_std]
//! RISC-V 64 指令编码器（`#![no_std]`）。
//!
//! 本 crate 把指令写成 Rust 值，再编码为机器码：
//!
//! - [`Instruction`]：32 位标准指令，覆盖 RV64I、M、A、Zicsr 以及常用特权指令；
//! - [`CompressedInstruction`]：16 位压缩指令，覆盖 C 扩展的常用子集；
//! - [`Encoded`]：统一保存 16/32 位编码结果，可从指针读取指令或写入字节缓冲区；
//! - [`Instruction::size`] / [`CompressedInstruction::size`]：查询指令在内存中的实际长度；
//! - [`reg`] / [`fence`]：寄存器别名与 `fence` 集合常量。
//!
//! 编码遵循 RISC-V 非特权与特权规范：
//!
//! - 立即数按指令语义传入（分支 / 跳转传相对于当前 `pc` 的字节偏移）；
//! - `Lui` / `Auipc` 的 `imm20` 是 `inst[31:12]` 原始 20 位值；
//! - 压缩指令的寄存器约束（`x8`–`x15`）在调试构建中通过 `debug_assert!` 检查；
//! - 立即数会被截断到对应字段宽度，调用者应保证其落在指令允许的范围内。
//!
//! ```ignore
//! use goethite_riscv64_asm::{Instruction, reg};
//!
//! let add = Instruction::Add { rd: reg::A0, rs1: reg::A0, rs2: reg::A1 };
//! assert_eq!(add.encode(), 0x00b5_0533);
//!
//! // 从当前 pc 读取一条指令，并推进到下一条指令。
//! let inst = unsafe { Encoded::read(pc as *const u8) };
//! pc += inst.size();
//! ```

use goethite_macros::bits;

/// 通用寄存器编号，合法范围为 `x0`–`x31`。
pub type Register = u8;

/// 常用寄存器编号与 ABI 别名。
pub mod reg {
    use super::Register;

    pub const X0: Register = 0;
    pub const X1: Register = 1;
    pub const X2: Register = 2;
    pub const X3: Register = 3;
    pub const X4: Register = 4;
    pub const X5: Register = 5;
    pub const X6: Register = 6;
    pub const X7: Register = 7;
    pub const X8: Register = 8;
    pub const X9: Register = 9;
    pub const X10: Register = 10;
    pub const X11: Register = 11;
    pub const X12: Register = 12;
    pub const X13: Register = 13;
    pub const X14: Register = 14;
    pub const X15: Register = 15;
    pub const X16: Register = 16;
    pub const X17: Register = 17;
    pub const X18: Register = 18;
    pub const X19: Register = 19;
    pub const X20: Register = 20;
    pub const X21: Register = 21;
    pub const X22: Register = 22;
    pub const X23: Register = 23;
    pub const X24: Register = 24;
    pub const X25: Register = 25;
    pub const X26: Register = 26;
    pub const X27: Register = 27;
    pub const X28: Register = 28;
    pub const X29: Register = 29;
    pub const X30: Register = 30;
    pub const X31: Register = 31;

    /// 零寄存器，读取恒为 0。
    pub const ZERO: Register = X0;
    /// 返回地址。
    pub const RA: Register = X1;
    /// 栈指针。
    pub const SP: Register = X2;
    /// 全局指针。
    pub const GP: Register = X3;
    /// 线程 / hart 指针。
    pub const TP: Register = X4;
    pub const T0: Register = X5;
    pub const T1: Register = X6;
    pub const T2: Register = X7;
    /// `s0` 同时是帧指针别名 `fp`。
    pub const S0: Register = X8;
    pub const FP: Register = X8;
    pub const S1: Register = X9;
    pub const A0: Register = X10;
    pub const A1: Register = X11;
    pub const A2: Register = X12;
    pub const A3: Register = X13;
    pub const A4: Register = X14;
    pub const A5: Register = X15;
    pub const A6: Register = X16;
    pub const A7: Register = X17;
    pub const S2: Register = X18;
    pub const S3: Register = X19;
    pub const S4: Register = X20;
    pub const S5: Register = X21;
    pub const S6: Register = X22;
    pub const S7: Register = X23;
    pub const S8: Register = X24;
    pub const S9: Register = X25;
    pub const S10: Register = X26;
    pub const S11: Register = X27;
    pub const T3: Register = X28;
    pub const T4: Register = X29;
    pub const T5: Register = X30;
    pub const T6: Register = X31;
}

/// `fence` 指令的前驱 / 后继集合位。
pub mod fence {
    /// 设备输入。
    pub const I: u8 = 1 << 3;
    /// 设备输出。
    pub const O: u8 = 1 << 2;
    /// 内存读。
    pub const R: u8 = 1 << 1;
    /// 内存写。
    pub const W: u8 = 1 << 0;
    /// 读写集合。
    pub const RW: u8 = R | W;
}

/// 原子内存操作宽度。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AmoWidth {
    /// `.w`，32 位。
    Word,
    /// `.d`，64 位。
    Double,
}

/// `AMO*` 指令的运算类型（不含 `lr` / `sc`）。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AmoOp {
    Add,
    Swap,
    Xor,
    And,
    Or,
    Min,
    Max,
    MinU,
    MaxU,
}

/// 32 位标准指令。
///
/// 所有 `offset` / `imm` 都按汇编语法传入；编码器负责把它们拆分到
/// 指令的各个比特字段。寄存器编号超出 `x0`–`x31` 属于调用者错误。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Instruction {
    // ===== U 型 =====
    Lui {
        rd: Register,
        imm20: u32,
    },
    Auipc {
        rd: Register,
        imm20: u32,
    },

    // ===== J 型 =====
    Jal {
        rd: Register,
        offset: i32,
    },

    // ===== I 型：跳转 =====
    Jalr {
        rd: Register,
        rs1: Register,
        offset: i32,
    },

    // ===== I 型：加载 =====
    Lb {
        rd: Register,
        rs1: Register,
        offset: i32,
    },
    Lh {
        rd: Register,
        rs1: Register,
        offset: i32,
    },
    Lw {
        rd: Register,
        rs1: Register,
        offset: i32,
    },
    Ld {
        rd: Register,
        rs1: Register,
        offset: i32,
    },
    Lbu {
        rd: Register,
        rs1: Register,
        offset: i32,
    },
    Lhu {
        rd: Register,
        rs1: Register,
        offset: i32,
    },
    Lwu {
        rd: Register,
        rs1: Register,
        offset: i32,
    },

    // ===== I 型：立即数运算 =====
    Addi {
        rd: Register,
        rs1: Register,
        imm: i32,
    },
    Slti {
        rd: Register,
        rs1: Register,
        imm: i32,
    },
    Sltiu {
        rd: Register,
        rs1: Register,
        imm: i32,
    },
    Xori {
        rd: Register,
        rs1: Register,
        imm: i32,
    },
    Ori {
        rd: Register,
        rs1: Register,
        imm: i32,
    },
    Andi {
        rd: Register,
        rs1: Register,
        imm: i32,
    },
    Slli {
        rd: Register,
        rs1: Register,
        shamt: u32,
    },
    Srli {
        rd: Register,
        rs1: Register,
        shamt: u32,
    },
    Srai {
        rd: Register,
        rs1: Register,
        shamt: u32,
    },

    // ===== I 型：64 位立即数运算 =====
    Addiw {
        rd: Register,
        rs1: Register,
        imm: i32,
    },
    Slliw {
        rd: Register,
        rs1: Register,
        shamt: u32,
    },
    Srliw {
        rd: Register,
        rs1: Register,
        shamt: u32,
    },
    Sraiw {
        rd: Register,
        rs1: Register,
        shamt: u32,
    },

    // ===== S 型：存储 =====
    Sb {
        rs2: Register,
        rs1: Register,
        offset: i32,
    },
    Sh {
        rs2: Register,
        rs1: Register,
        offset: i32,
    },
    Sw {
        rs2: Register,
        rs1: Register,
        offset: i32,
    },
    Sd {
        rs2: Register,
        rs1: Register,
        offset: i32,
    },

    // ===== B 型：条件分支 =====
    Beq {
        rs1: Register,
        rs2: Register,
        offset: i32,
    },
    Bne {
        rs1: Register,
        rs2: Register,
        offset: i32,
    },
    Blt {
        rs1: Register,
        rs2: Register,
        offset: i32,
    },
    Bge {
        rs1: Register,
        rs2: Register,
        offset: i32,
    },
    Bltu {
        rs1: Register,
        rs2: Register,
        offset: i32,
    },
    Bgeu {
        rs1: Register,
        rs2: Register,
        offset: i32,
    },

    // ===== R 型：寄存器运算 =====
    Add {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Sub {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Sll {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Slt {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Sltu {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Xor {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Srl {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Sra {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Or {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    And {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Addw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Subw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Sllw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Srlw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Sraw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },

    // ===== M 扩展 =====
    Mul {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Mulh {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Mulhsu {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Mulhu {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Div {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Divu {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Rem {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Remu {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Mulw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Divw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Divuw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Remw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },
    Remuw {
        rd: Register,
        rs1: Register,
        rs2: Register,
    },

    // ===== A 扩展 =====
    Lr {
        width: AmoWidth,
        rd: Register,
        rs1: Register,
        aq: bool,
        rl: bool,
    },
    Sc {
        width: AmoWidth,
        rd: Register,
        rs1: Register,
        rs2: Register,
        aq: bool,
        rl: bool,
    },
    Amo {
        op: AmoOp,
        width: AmoWidth,
        rd: Register,
        rs1: Register,
        rs2: Register,
        aq: bool,
        rl: bool,
    },

    // ===== 内存序 =====
    Fence {
        fm: u8,
        predecessor: u8,
        successor: u8,
    },
    FenceI,

    // ===== CSR =====
    Csrrw {
        rd: Register,
        rs1: Register,
        csr: u16,
    },
    Csrrs {
        rd: Register,
        rs1: Register,
        csr: u16,
    },
    Csrrc {
        rd: Register,
        rs1: Register,
        csr: u16,
    },
    Csrrwi {
        rd: Register,
        uimm: u8,
        csr: u16,
    },
    Csrrsi {
        rd: Register,
        uimm: u8,
        csr: u16,
    },
    Csrrci {
        rd: Register,
        uimm: u8,
        csr: u16,
    },

    // ===== 系统 / 特权 =====
    Ecall,
    Ebreak,
    Mret,
    Sret,
    Wfi,
    SfenceVma {
        rs1: Register,
        rs2: Register,
    },
}

impl Instruction {
    /// 32 位标准指令在内存中的长度（字节）。
    pub const SIZE: usize = 4;

    /// 该指令在内存中的实际长度。
    #[inline]
    pub const fn size(&self) -> usize {
        Self::SIZE
    }

    /// 编码为 32 位指令字。
    #[inline]
    pub fn encode(&self) -> u32 {
        use Instruction::*;

        match *self {
            // ===== U 型 =====
            Lui { rd, imm20 } => u_type(op::LUI, rd, imm20),
            Auipc { rd, imm20 } => u_type(op::AUIPC, rd, imm20),

            // ===== J 型 =====
            Jal { rd, offset } => j_type(op::JAL, rd, offset),

            // ===== I 型 =====
            Jalr { rd, rs1, offset } => i_type(op::JALR, rd, 0b000, rs1, offset),

            Lb { rd, rs1, offset } => i_type(op::LOAD, rd, 0b000, rs1, offset),
            Lh { rd, rs1, offset } => i_type(op::LOAD, rd, 0b001, rs1, offset),
            Lw { rd, rs1, offset } => i_type(op::LOAD, rd, 0b010, rs1, offset),
            Ld { rd, rs1, offset } => i_type(op::LOAD, rd, 0b011, rs1, offset),
            Lbu { rd, rs1, offset } => i_type(op::LOAD, rd, 0b100, rs1, offset),
            Lhu { rd, rs1, offset } => i_type(op::LOAD, rd, 0b101, rs1, offset),
            Lwu { rd, rs1, offset } => i_type(op::LOAD, rd, 0b110, rs1, offset),

            Addi { rd, rs1, imm } => i_type(op::OP_IMM, rd, 0b000, rs1, imm),
            Slti { rd, rs1, imm } => i_type(op::OP_IMM, rd, 0b010, rs1, imm),
            Sltiu { rd, rs1, imm } => i_type(op::OP_IMM, rd, 0b011, rs1, imm),
            Xori { rd, rs1, imm } => i_type(op::OP_IMM, rd, 0b100, rs1, imm),
            Ori { rd, rs1, imm } => i_type(op::OP_IMM, rd, 0b110, rs1, imm),
            Andi { rd, rs1, imm } => i_type(op::OP_IMM, rd, 0b111, rs1, imm),
            Slli { rd, rs1, shamt } => i_type(op::OP_IMM, rd, 0b001, rs1, shamt as i32),
            Srli { rd, rs1, shamt } => i_type(op::OP_IMM, rd, 0b101, rs1, shamt as i32),
            Srai { rd, rs1, shamt } => {
                i_type(op::OP_IMM, rd, 0b101, rs1, (0b010000 << 6) | shamt as i32)
            }

            Addiw { rd, rs1, imm } => i_type(op::OP_IMM_32, rd, 0b000, rs1, imm),
            Slliw { rd, rs1, shamt } => i_type(op::OP_IMM_32, rd, 0b001, rs1, shamt as i32),
            Srliw { rd, rs1, shamt } => i_type(op::OP_IMM_32, rd, 0b101, rs1, shamt as i32),
            Sraiw { rd, rs1, shamt } => i_type(
                op::OP_IMM_32,
                rd,
                0b101,
                rs1,
                (0b0100000 << 5) | shamt as i32,
            ),

            // ===== S 型 =====
            Sb { rs2, rs1, offset } => s_type(op::STORE, 0b000, rs1, rs2, offset),
            Sh { rs2, rs1, offset } => s_type(op::STORE, 0b001, rs1, rs2, offset),
            Sw { rs2, rs1, offset } => s_type(op::STORE, 0b010, rs1, rs2, offset),
            Sd { rs2, rs1, offset } => s_type(op::STORE, 0b011, rs1, rs2, offset),

            // ===== B 型 =====
            Beq { rs1, rs2, offset } => b_type(op::BRANCH, 0b000, rs1, rs2, offset),
            Bne { rs1, rs2, offset } => b_type(op::BRANCH, 0b001, rs1, rs2, offset),
            Blt { rs1, rs2, offset } => b_type(op::BRANCH, 0b100, rs1, rs2, offset),
            Bge { rs1, rs2, offset } => b_type(op::BRANCH, 0b101, rs1, rs2, offset),
            Bltu { rs1, rs2, offset } => b_type(op::BRANCH, 0b110, rs1, rs2, offset),
            Bgeu { rs1, rs2, offset } => b_type(op::BRANCH, 0b111, rs1, rs2, offset),

            // ===== R 型 =====
            Add { rd, rs1, rs2 } => r_type(op::OP, rd, 0b000, rs1, rs2, 0b0000000),
            Sub { rd, rs1, rs2 } => r_type(op::OP, rd, 0b000, rs1, rs2, 0b0100000),
            Sll { rd, rs1, rs2 } => r_type(op::OP, rd, 0b001, rs1, rs2, 0b0000000),
            Slt { rd, rs1, rs2 } => r_type(op::OP, rd, 0b010, rs1, rs2, 0b0000000),
            Sltu { rd, rs1, rs2 } => r_type(op::OP, rd, 0b011, rs1, rs2, 0b0000000),
            Xor { rd, rs1, rs2 } => r_type(op::OP, rd, 0b100, rs1, rs2, 0b0000000),
            Srl { rd, rs1, rs2 } => r_type(op::OP, rd, 0b101, rs1, rs2, 0b0000000),
            Sra { rd, rs1, rs2 } => r_type(op::OP, rd, 0b101, rs1, rs2, 0b0100000),
            Or { rd, rs1, rs2 } => r_type(op::OP, rd, 0b110, rs1, rs2, 0b0000000),
            And { rd, rs1, rs2 } => r_type(op::OP, rd, 0b111, rs1, rs2, 0b0000000),
            Addw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b000, rs1, rs2, 0b0000000),
            Subw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b000, rs1, rs2, 0b0100000),
            Sllw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b001, rs1, rs2, 0b0000000),
            Srlw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b101, rs1, rs2, 0b0000000),
            Sraw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b101, rs1, rs2, 0b0100000),

            // ===== M 扩展 =====
            Mul { rd, rs1, rs2 } => r_type(op::OP, rd, 0b000, rs1, rs2, 0b0000001),
            Mulh { rd, rs1, rs2 } => r_type(op::OP, rd, 0b001, rs1, rs2, 0b0000001),
            Mulhsu { rd, rs1, rs2 } => r_type(op::OP, rd, 0b010, rs1, rs2, 0b0000001),
            Mulhu { rd, rs1, rs2 } => r_type(op::OP, rd, 0b011, rs1, rs2, 0b0000001),
            Div { rd, rs1, rs2 } => r_type(op::OP, rd, 0b100, rs1, rs2, 0b0000001),
            Divu { rd, rs1, rs2 } => r_type(op::OP, rd, 0b101, rs1, rs2, 0b0000001),
            Rem { rd, rs1, rs2 } => r_type(op::OP, rd, 0b110, rs1, rs2, 0b0000001),
            Remu { rd, rs1, rs2 } => r_type(op::OP, rd, 0b111, rs1, rs2, 0b0000001),
            Mulw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b000, rs1, rs2, 0b0000001),
            Divw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b100, rs1, rs2, 0b0000001),
            Divuw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b101, rs1, rs2, 0b0000001),
            Remw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b110, rs1, rs2, 0b0000001),
            Remuw { rd, rs1, rs2 } => r_type(op::OP_32, rd, 0b111, rs1, rs2, 0b0000001),

            // ===== A 扩展 =====
            Lr {
                width,
                rd,
                rs1,
                aq,
                rl,
            } => amo_type(0b00010, width, rd, rs1, 0, aq, rl),
            Sc {
                width,
                rd,
                rs1,
                rs2,
                aq,
                rl,
            } => amo_type(0b00011, width, rd, rs1, rs2, aq, rl),
            Amo {
                op,
                width,
                rd,
                rs1,
                rs2,
                aq,
                rl,
            } => {
                let funct5 = match op {
                    AmoOp::Add => 0b00000,
                    AmoOp::Swap => 0b00001,
                    AmoOp::Xor => 0b00100,
                    AmoOp::And => 0b01100,
                    AmoOp::Or => 0b01000,
                    AmoOp::Min => 0b10000,
                    AmoOp::Max => 0b10100,
                    AmoOp::MinU => 0b11000,
                    AmoOp::MaxU => 0b11100,
                };
                amo_type(funct5, width, rd, rs1, rs2, aq, rl)
            }

            // ===== 内存序 =====
            Fence {
                fm,
                predecessor,
                successor,
            } => {
                let imm = ((fm as u32 & 0xf) << 8)
                    | ((predecessor as u32 & 0xf) << 4)
                    | (successor as u32 & 0xf);
                i_type(op::MISC_MEM, 0, 0b000, 0, imm as i32)
            }
            FenceI => i_type(op::MISC_MEM, 0, 0b001, 0, 0),

            // ===== CSR =====
            Csrrw { rd, rs1, csr } => i_type(op::SYSTEM, rd, 0b001, rs1, csr as i32),
            Csrrs { rd, rs1, csr } => i_type(op::SYSTEM, rd, 0b010, rs1, csr as i32),
            Csrrc { rd, rs1, csr } => i_type(op::SYSTEM, rd, 0b011, rs1, csr as i32),
            Csrrwi { rd, uimm, csr } => i_type(op::SYSTEM, rd, 0b101, uimm, csr as i32),
            Csrrsi { rd, uimm, csr } => i_type(op::SYSTEM, rd, 0b110, uimm, csr as i32),
            Csrrci { rd, uimm, csr } => i_type(op::SYSTEM, rd, 0b111, uimm, csr as i32),

            // ===== 系统 / 特权 =====
            Ecall => 0x0000_0073,
            Ebreak => 0x0010_0073,
            Mret => 0x3020_0073,
            Sret => 0x1020_0073,
            Wfi => 0x1050_0073,
            SfenceVma { rs1, rs2 } => r_type(op::SYSTEM, 0, 0b000, rs1, rs2, 0b0001001),
        }
    }
}

/// 16 位压缩指令（C 扩展常用子集）。
///
/// 使用 `x8`–`x15` 的变体对应规范中的 `rd'` / `rs1'` / `rs2'`，
/// 传入其它寄存器会在调试构建中触发断言。以 `sp` 为基址的变体
/// （`Lwsp` / `Ldsp` / `Swsp` / `Sdsp`）直接接受任意寄存器，编码器
/// 按规范省略隐含的 `sp`。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CompressedInstruction {
    /// `c.nop`
    Nop,
    /// `c.addi`，6 位有符号立即数。
    Addi { rd: Register, imm: i32 },
    /// `c.addiw`（RV64），6 位有符号立即数。
    Addiw { rd: Register, imm: i32 },
    /// `c.li`，6 位有符号立即数。
    Li { rd: Register, imm: i32 },
    /// `c.lui`，`imm` 是 `nzimm[17:12]` 的 6 位有符号值。
    Lui { rd: Register, imm: i32 },
    /// `c.addi16sp`，立即数是 16 的非零倍数。
    Addi16sp { imm: i32 },
    /// `c.addi4spn`，立即数是 4 的非零倍数。
    Addi4spn { rd: Register, imm: u32 },

    /// `c.slli`。
    Slli { rd: Register, shamt: u32 },
    /// `c.srli`。
    Srli { rd: Register, shamt: u32 },
    /// `c.srai`。
    Srai { rd: Register, shamt: u32 },
    /// `c.andi`，6 位有符号立即数。
    Andi { rd: Register, imm: i32 },

    /// `c.mv`
    Mv { rd: Register, rs2: Register },
    /// `c.add`
    Add { rd: Register, rs2: Register },
    /// `c.sub`
    Sub { rd: Register, rs2: Register },
    /// `c.xor`
    Xor { rd: Register, rs2: Register },
    /// `c.or`
    Or { rd: Register, rs2: Register },
    /// `c.and`
    And { rd: Register, rs2: Register },
    /// `c.subw`（RV64）
    Subw { rd: Register, rs2: Register },
    /// `c.addw`（RV64）
    Addw { rd: Register, rs2: Register },

    /// `c.lw`
    Lw {
        rd: Register,
        rs1: Register,
        offset: u32,
    },
    /// `c.ld`（RV64）
    Ld {
        rd: Register,
        rs1: Register,
        offset: u32,
    },
    /// `c.sw`
    Sw {
        rs2: Register,
        rs1: Register,
        offset: u32,
    },
    /// `c.sd`（RV64）
    Sd {
        rs2: Register,
        rs1: Register,
        offset: u32,
    },
    /// `c.lwsp`
    Lwsp { rd: Register, offset: u32 },
    /// `c.ldsp`（RV64）
    Ldsp { rd: Register, offset: u32 },
    /// `c.swsp`
    Swsp { rs2: Register, offset: u32 },
    /// `c.sdsp`（RV64）
    Sdsp { rs2: Register, offset: u32 },

    /// `c.beqz`
    Beqz { rs1: Register, offset: i32 },
    /// `c.bnez`
    Bnez { rs1: Register, offset: i32 },
    /// `c.j`
    J { offset: i32 },
    /// `c.jr`
    Jr { rs1: Register },
    /// `c.jalr`
    Jalr { rs1: Register },
    /// `c.ebreak`
    Ebreak,
}

impl CompressedInstruction {
    /// 16 位压缩指令在内存中的长度（字节）。
    pub const SIZE: usize = 2;

    /// 该指令在内存中的实际长度。
    #[inline]
    pub const fn size(&self) -> usize {
        Self::SIZE
    }

    /// 编码为 16 位压缩指令字。
    #[inline]
    pub fn encode(&self) -> u16 {
        use CompressedInstruction::*;

        match *self {
            Nop => ci_type(0b01, 0b000, 0, 0),
            Addi { rd, imm } => ci_type(0b01, 0b000, rd, imm as u16 & 0x3f),
            Addiw { rd, imm } => ci_type(0b01, 0b001, rd, imm as u16 & 0x3f),
            Li { rd, imm } => ci_type(0b01, 0b010, rd, imm as u16 & 0x3f),
            Lui { rd, imm } => ci_type(0b01, 0b011, rd, imm as u16 & 0x3f),
            Addi16sp { imm } => {
                let u = imm as u16;
                let imm6 = (((u >> 9) & 1) << 5)
                    | (((u >> 4) & 1) << 4)
                    | (((u >> 6) & 1) << 3)
                    | (((u >> 7) & 0x3) << 1)
                    | ((u >> 5) & 1);
                ci_type(0b01, 0b011, reg::SP, imm6)
            }
            Addi4spn { rd, imm } => {
                let u = imm as u16;
                let imm8 = (((u >> 4) & 0x3) << 6)
                    | (((u >> 6) & 0xf) << 2)
                    | (((u >> 2) & 1) << 1)
                    | ((u >> 3) & 1);
                ciw_type(0b000, rd, imm8)
            }

            Slli { rd, shamt } => ci_type(0b10, 0b000, rd, shamt as u16 & 0x3f),
            Srli { rd, shamt } => cb_imm_type(0b00, rd, shamt),
            Srai { rd, shamt } => cb_imm_type(0b01, rd, shamt),
            Andi { rd, imm } => cb_imm_type(0b10, rd, imm as u32),

            Mv { rd, rs2 } => cr_type(0b1000, rd, rs2),
            Add { rd, rs2 } => cr_type(0b1001, rd, rs2),
            Sub { rd, rs2 } => ca_type(0b100011, 0b00, rd, rs2),
            Xor { rd, rs2 } => ca_type(0b100011, 0b01, rd, rs2),
            Or { rd, rs2 } => ca_type(0b100011, 0b10, rd, rs2),
            And { rd, rs2 } => ca_type(0b100011, 0b11, rd, rs2),
            Subw { rd, rs2 } => ca_type(0b100111, 0b00, rd, rs2),
            Addw { rd, rs2 } => ca_type(0b100111, 0b01, rd, rs2),

            Lw { rd, rs1, offset } => cl_type(0b010, rd, rs1, offset, false),
            Ld { rd, rs1, offset } => cl_type(0b011, rd, rs1, offset, true),
            Sw { rs2, rs1, offset } => cs_type(0b110, rs2, rs1, offset, false),
            Sd { rs2, rs1, offset } => cs_type(0b111, rs2, rs1, offset, true),
            Lwsp { rd, offset } => {
                let u = offset as u16;
                let imm6 = (((u >> 5) & 1) << 5) | (((u >> 2) & 0x7) << 2) | ((u >> 6) & 0x3);
                ci_type(0b10, 0b010, rd, imm6)
            }
            Ldsp { rd, offset } => {
                let u = offset as u16;
                let imm6 = (((u >> 5) & 1) << 5) | (((u >> 3) & 0x3) << 3) | ((u >> 6) & 0x7);
                ci_type(0b10, 0b011, rd, imm6)
            }
            Swsp { rs2, offset } => {
                let u = offset as u16;
                let imm6 = (((u >> 2) & 0xf) << 2) | ((u >> 6) & 0x3);
                css_type(0b110, rs2, imm6)
            }
            Sdsp { rs2, offset } => {
                let u = offset as u16;
                let imm6 = (((u >> 3) & 0x7) << 3) | ((u >> 6) & 0x7);
                css_type(0b111, rs2, imm6)
            }

            Beqz { rs1, offset } => cb_type(0b110, rs1, offset),
            Bnez { rs1, offset } => cb_type(0b111, rs1, offset),
            J { offset } => cj_type(0b101, offset),
            Jr { rs1 } => cr_type(0b1000, rs1, 0),
            Jalr { rs1 } => cr_type(0b1001, rs1, 0),
            Ebreak => 0x9002,
        }
    }
}

/// 统一表示 16 位或 32 位指令编码。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Encoded {
    /// 16 位压缩指令。
    Compressed(u16),
    /// 32 位标准指令。
    Standard(u32),
}

impl Encoded {
    /// 从指针读取一条指令，并自动区分 16 位压缩指令与 32 位标准指令。
    ///
    /// 读取 RISC-V 规范定义的低两位：不等于 `0b11` 时为 16 位压缩指令，
    /// 否则为 32 位标准指令。指令按小端序解释。
    ///
    /// # Safety
    ///
    /// - `ptr` 必须指向至少 2 字节的可读内存；
    /// - 若该地址的低两位为 `0b11`，还必须保证其后 4 字节全部可读。
    ///
    /// 该函数使用非对齐读取，因此 `ptr` 不要求按指令宽度对齐。
    #[inline]
    pub unsafe fn read(ptr: *const u8) -> Self {
        // 先读半字，低两位不是 0b11 就说明这是一条 16 位压缩指令。
        let half = unsafe { core::ptr::read_unaligned(ptr.cast::<u16>()) };
        let half = u16::from_le(half);
        if half & 0b11 != 0b11 {
            return Encoded::Compressed(half);
        }

        let word = unsafe { core::ptr::read_unaligned(ptr.cast::<u32>()) };
        Encoded::Standard(u32::from_le(word))
    }

    /// 编码长度（字节）。
    #[inline]
    pub const fn size(self) -> usize {
        match self {
            Encoded::Compressed(_) => CompressedInstruction::SIZE,
            Encoded::Standard(_) => Instruction::SIZE,
        }
    }

    /// 是否为 16 位压缩指令。
    #[inline]
    pub const fn is_compressed(self) -> bool {
        matches!(self, Encoded::Compressed(_))
    }

    /// 把指令字按小端序写入 `out`，返回写入的字节数；空间不足时返回 `None`。
    #[inline]
    pub fn write_to(self, out: &mut [u8]) -> Option<usize> {
        let size = self.size();
        if out.len() < size {
            return None;
        }
        match self {
            Encoded::Compressed(value) => out[..2].copy_from_slice(&value.to_le_bytes()),
            Encoded::Standard(value) => out[..4].copy_from_slice(&value.to_le_bytes()),
        }
        Some(size)
    }
}

impl From<Instruction> for Encoded {
    fn from(value: Instruction) -> Self {
        Encoded::Standard(value.encode())
    }
}

impl From<CompressedInstruction> for Encoded {
    fn from(value: CompressedInstruction) -> Self {
        Encoded::Compressed(value.encode())
    }
}

/// 指令 opcode 字段。
mod op {
    pub const LOAD: u32 = 0b0000011;
    pub const MISC_MEM: u32 = 0b0001111;
    pub const OP_IMM: u32 = 0b0010011;
    pub const AUIPC: u32 = 0b0010111;
    pub const OP_IMM_32: u32 = 0b0011011;
    pub const STORE: u32 = 0b0100011;
    pub const OP: u32 = 0b0110011;
    pub const LUI: u32 = 0b0110111;
    pub const OP_32: u32 = 0b0111011;
    pub const BRANCH: u32 = 0b1100011;
    pub const JALR: u32 = 0b1100111;
    pub const JAL: u32 = 0b1101111;
    pub const SYSTEM: u32 = 0b1110011;
    pub const AMO: u32 = 0b0101111;
}

// ===== 32 位指令格式 =====

bits! {
    pub type RTypeInstruction : u32 {
        opcode: 0 => 6,
        rd: 7 => 11,
        funct3: 12 => 14,
        rs1: 15 => 19,
        rs2: 20 => 24,
        funct7: 25 => 31,
    }
}

bits! {
    pub type ITypeInstruction : u32 {
        opcode: 0 => 6,
        rd: 7 => 11,
        funct3: 12 => 14,
        rs1: 15 => 19,
        imm: 20 => 31,
    }
}

bits! {
    pub type STypeInstruction : u32 {
        opcode: 0 => 6,
        imm4_0: 7 => 11,
        funct3: 12 => 14,
        rs1: 15 => 19,
        rs2: 20 => 24,
        imm11_5: 25 => 31,
    }
}

bits! {
    pub type BTypeInstruction : u32 {
        opcode: 0 => 6,
        imm11: 7,
        imm4_1: 8 => 11,
        funct3: 12 => 14,
        rs1: 15 => 19,
        rs2: 20 => 24,
        imm10_5: 25 => 30,
        imm12: 31,
    }
}

bits! {
    pub type UTypeInstruction : u32 {
        opcode: 0 => 6,
        rd: 7 => 11,
        imm: 12 => 31,
    }
}

bits! {
    pub type JTypeInstruction : u32 {
        opcode: 0 => 6,
        rd: 7 => 11,
        imm19_12: 12 => 19,
        imm11: 20,
        imm10_1: 21 => 30,
        imm20: 31,
    }
}

bits! {
    pub type ATypeInstruction : u32 {
        opcode: 0 => 6,
        rd: 7 => 11,
        funct3: 12 => 14,
        rs1: 15 => 19,
        rs2: 20 => 24,
        rl: 25,
        aq: 26,
        funct5: 27 => 31,
    }
}

// ===== 16 位压缩指令格式 =====

bits! {
    pub type CRTypeInstruction : u16 {
        opcode: 0 => 1,
        rs2: 2 => 6,
        rs1: 7 => 11,
        funct4: 12 => 15,
    }
}

// CI：bit12 与 bit6:2 共同组成 6 位立即数，bit11:7 是 rd。
bits! {
    pub type CITypeInstruction : u16 {
        opcode: 0 => 1,
        imm_lo: 2 => 6,
        rd: 7 => 11,
        imm_hi: 12,
        funct3: 13 => 15,
    }
}

// CSS：6 位立即数位于 bit12:7，寄存器位于 bit6:2。
bits! {
    pub type CSSTypeInstruction : u16 {
        opcode: 0 => 1,
        rs2: 2 => 6,
        imm: 7 => 12,
        funct3: 13 => 15,
    }
}

// CIW：8 位立即数位于 bit12:5，rd' 位于 bit4:2。
bits! {
    pub type CIWTypeInstruction : u16 {
        opcode: 0 => 1,
        rd: 2 => 4,
        imm: 5 => 12,
        funct3: 13 => 15,
    }
}

// CL / CS：bit12:10 是 offset[5:3]，bit6 与 bit5 承载剩余低位/高位。
bits! {
    pub type CLTypeInstruction : u16 {
        opcode: 0 => 1,
        rd: 2 => 4,
        imm_bit5: 5,
        imm_bit6: 6,
        rs1: 7 => 9,
        imm5_3: 10 => 12,
        funct3: 13 => 15,
    }
}

bits! {
    pub type CSTypeInstruction : u16 {
        opcode: 0 => 1,
        rs2: 2 => 4,
        imm_bit5: 5,
        imm_bit6: 6,
        rs1: 7 => 9,
        imm5_3: 10 => 12,
        funct3: 13 => 15,
    }
}

// CA：压缩寄存器运算。
bits! {
    pub type CATypeInstruction : u16 {
        opcode: 0 => 1,
        rs2: 2 => 4,
        funct2: 5 => 6,
        rs1: 7 => 9,
        funct6: 10 => 15,
    }
}

// CB：压缩分支，offset 位分散在 bit12:10 与 bit6:2。
bits! {
    pub type CBTypeInstruction : u16 {
        opcode: 0 => 1,
        imm5: 2,
        imm2_1: 3 => 4,
        imm7_6: 5 => 6,
        rs1: 7 => 9,
        imm4_3: 10 => 11,
        imm12: 12,
        funct3: 13 => 15,
    }
}

// CJ：压缩跳转，offset 位分散在 bit12:2。
bits! {
    pub type CJTypeInstruction : u16 {
        opcode: 0 => 1,
        imm5: 2,
        imm3_1: 3 => 5,
        imm7: 6,
        imm6: 7,
        imm10: 8,
        imm9_8: 9 => 10,
        imm4: 11,
        imm11: 12,
        funct3: 13 => 15,
    }
}

// ===== 编码辅助函数 =====

/// 取 5 位寄存器编号。
#[inline(always)]
fn reg5(reg: Register) -> u32 {
    debug_assert!(reg < 32, "寄存器编号超出 x0..x31: {reg}");
    (reg & 0x1f) as u32
}

/// 压缩指令中 3 位寄存器编号（`x8`–`x15`）。
#[inline(always)]
fn creg3(reg: Register) -> u16 {
    debug_assert!((8..=15).contains(&reg), "压缩指令仅接受 x8..x15: {reg}");
    (reg & 0x7) as u16
}

/// R 型编码。
#[inline]
fn r_type(
    opcode: u32,
    rd: Register,
    funct3: u32,
    rs1: Register,
    rs2: Register,
    funct7: u32,
) -> u32 {
    let mut inst = RTypeInstruction::new();
    inst.set_opcode(opcode);
    inst.set_rd(reg5(rd));
    inst.set_funct3(funct3);
    inst.set_rs1(reg5(rs1));
    inst.set_rs2(reg5(rs2));
    inst.set_funct7(funct7);
    inst.into()
}

/// I 型编码。
#[inline]
fn i_type(opcode: u32, rd: Register, funct3: u32, rs1: Register, imm: i32) -> u32 {
    let mut inst = ITypeInstruction::new();
    inst.set_opcode(opcode);
    inst.set_rd(reg5(rd));
    inst.set_funct3(funct3);
    inst.set_rs1(reg5(rs1));
    inst.set_imm((imm as u32) & 0xfff);
    inst.into()
}

/// S 型编码。
#[inline]
fn s_type(opcode: u32, funct3: u32, rs1: Register, rs2: Register, offset: i32) -> u32 {
    let imm = offset as u32;
    let mut inst = STypeInstruction::new();
    inst.set_opcode(opcode);
    inst.set_funct3(funct3);
    inst.set_rs1(reg5(rs1));
    inst.set_rs2(reg5(rs2));
    inst.set_imm4_0(imm & 0x1f);
    inst.set_imm11_5((imm >> 5) & 0x7f);
    inst.into()
}

/// B 型编码。`offset` 是相对当前 `pc` 的字节偏移。
#[inline]
fn b_type(opcode: u32, funct3: u32, rs1: Register, rs2: Register, offset: i32) -> u32 {
    debug_assert_eq!(offset & 1, 0, "分支偏移必须 2 字节对齐");
    let imm = offset as u32;
    let mut inst = BTypeInstruction::new();
    inst.set_opcode(opcode);
    inst.set_funct3(funct3);
    inst.set_rs1(reg5(rs1));
    inst.set_rs2(reg5(rs2));
    inst.set_imm11(((imm >> 11) & 1) != 0);
    inst.set_imm4_1((imm >> 1) & 0xf);
    inst.set_imm10_5((imm >> 5) & 0x3f);
    inst.set_imm12(((imm >> 12) & 1) != 0);
    inst.into()
}

/// U 型编码。`imm20` 是 `inst[31:12]` 原始值。
#[inline]
fn u_type(opcode: u32, rd: Register, imm20: u32) -> u32 {
    let mut inst = UTypeInstruction::new();
    inst.set_opcode(opcode);
    inst.set_rd(reg5(rd));
    inst.set_imm(imm20 & 0xf_ffff);
    inst.into()
}

/// J 型编码。`offset` 是相对当前 `pc` 的字节偏移。
#[inline]
fn j_type(opcode: u32, rd: Register, offset: i32) -> u32 {
    debug_assert_eq!(offset & 1, 0, "跳转偏移必须 2 字节对齐");
    let imm = offset as u32;
    let mut inst = JTypeInstruction::new();
    inst.set_opcode(opcode);
    inst.set_rd(reg5(rd));
    inst.set_imm19_12((imm >> 12) & 0xff);
    inst.set_imm11(((imm >> 11) & 1) != 0);
    inst.set_imm10_1((imm >> 1) & 0x3ff);
    inst.set_imm20(((imm >> 20) & 1) != 0);
    inst.into()
}

/// A 扩展编码。
#[inline]
fn amo_type(
    funct5: u32,
    width: AmoWidth,
    rd: Register,
    rs1: Register,
    rs2: Register,
    aq: bool,
    rl: bool,
) -> u32 {
    let funct3 = match width {
        AmoWidth::Word => 0b010,
        AmoWidth::Double => 0b011,
    };
    let mut inst = ATypeInstruction::new();
    inst.set_opcode(op::AMO);
    inst.set_rd(reg5(rd));
    inst.set_funct3(funct3);
    inst.set_rs1(reg5(rs1));
    inst.set_rs2(reg5(rs2));
    inst.set_rl(rl);
    inst.set_aq(aq);
    inst.set_funct5(funct5);
    inst.into()
}

/// CR 型压缩编码。
#[inline]
fn cr_type(funct4: u16, rs1: Register, rs2: Register) -> u16 {
    let mut inst = CRTypeInstruction::new();
    inst.set_opcode(0b10);
    inst.set_funct4(funct4);
    inst.set_rs1(reg5(rs1) as u16);
    inst.set_rs2(reg5(rs2) as u16);
    inst.into()
}

/// CI 型压缩编码。`imm6` 是 6 位立即数字段。
///
/// `opcode` 是 2 位象限：`c.addi` 等属于象限 1，`c.slli` 与
/// `c.lwsp` / `c.ldsp` 属于象限 2。
#[inline]
fn ci_type(opcode: u16, funct3: u16, rd: Register, imm6: u16) -> u16 {
    let mut inst = CITypeInstruction::new();
    inst.set_opcode(opcode);
    inst.set_funct3(funct3);
    inst.set_rd(reg5(rd) as u16);
    inst.set_imm_hi(((imm6 >> 5) & 1) != 0);
    inst.set_imm_lo(imm6 & 0x1f);
    inst.into()
}

/// CSS 型压缩编码。
#[inline]
fn css_type(funct3: u16, rs2: Register, imm6: u16) -> u16 {
    let mut inst = CSSTypeInstruction::new();
    inst.set_opcode(0b10);
    inst.set_funct3(funct3);
    inst.set_rs2(reg5(rs2) as u16);
    inst.set_imm(imm6 & 0x3f);
    inst.into()
}

/// CIW 型压缩编码。
#[inline]
fn ciw_type(funct3: u16, rd: Register, imm8: u16) -> u16 {
    let mut inst = CIWTypeInstruction::new();
    inst.set_opcode(0b00);
    inst.set_funct3(funct3);
    inst.set_rd(creg3(rd));
    inst.set_imm(imm8 & 0xff);
    inst.into()
}

/// CA 型压缩编码。
#[inline]
fn ca_type(funct6: u16, funct2: u16, rs1: Register, rs2: Register) -> u16 {
    let mut inst = CATypeInstruction::new();
    inst.set_opcode(0b01);
    inst.set_funct6(funct6);
    inst.set_funct2(funct2);
    inst.set_rs1(creg3(rs1));
    inst.set_rs2(creg3(rs2));
    inst.into()
}

/// `c.srli` / `c.srai` / `c.andi` 共用编码：`funct2` 选择操作。
#[inline]
fn cb_imm_type(funct2: u16, rs1: Register, imm: u32) -> u16 {
    let imm = imm as u16 & 0x3f;
    (0b100 << 13)
        | (((imm >> 5) & 1) << 12)
        | ((funct2 & 0x3) << 10)
        | (creg3(rs1) << 7)
        | ((imm & 0x1f) << 2)
        | 0b01
}

/// CL 型压缩加载编码。
#[inline]
fn cl_type(funct3: u16, rd: Register, rs1: Register, offset: u32, is_double: bool) -> u16 {
    debug_assert_eq!(
        offset & if is_double { 7 } else { 3 },
        0,
        "压缩加载偏移未按数据宽度对齐"
    );
    let mut inst = CLTypeInstruction::new();
    inst.set_opcode(0b00);
    inst.set_funct3(funct3);
    inst.set_rd(creg3(rd));
    inst.set_rs1(creg3(rs1));
    inst.set_imm5_3(((offset >> 3) & 0x7) as u16);
    if is_double {
        inst.set_imm_bit5(((offset >> 7) & 1) != 0);
        inst.set_imm_bit6(((offset >> 6) & 1) != 0);
    } else {
        inst.set_imm_bit5(((offset >> 6) & 1) != 0);
        inst.set_imm_bit6(((offset >> 2) & 1) != 0);
    }
    inst.into()
}

/// CS 型压缩存储编码。
#[inline]
fn cs_type(funct3: u16, rs2: Register, rs1: Register, offset: u32, is_double: bool) -> u16 {
    debug_assert_eq!(
        offset & if is_double { 7 } else { 3 },
        0,
        "压缩存储偏移未按数据宽度对齐"
    );
    let mut inst = CSTypeInstruction::new();
    inst.set_opcode(0b00);
    inst.set_funct3(funct3);
    inst.set_rs2(creg3(rs2));
    inst.set_rs1(creg3(rs1));
    inst.set_imm5_3(((offset >> 3) & 0x7) as u16);
    if is_double {
        inst.set_imm_bit5(((offset >> 7) & 1) != 0);
        inst.set_imm_bit6(((offset >> 6) & 1) != 0);
    } else {
        inst.set_imm_bit5(((offset >> 6) & 1) != 0);
        inst.set_imm_bit6(((offset >> 2) & 1) != 0);
    }
    inst.into()
}

/// CB 型压缩分支编码。
#[inline]
fn cb_type(funct3: u16, rs1: Register, offset: i32) -> u16 {
    debug_assert_eq!(offset & 1, 0, "压缩分支偏移必须 2 字节对齐");
    let imm = offset as u16;
    let mut inst = CBTypeInstruction::new();
    inst.set_opcode(0b01);
    inst.set_funct3(funct3);
    inst.set_rs1(creg3(rs1));
    inst.set_imm5(((imm >> 5) & 1) != 0);
    inst.set_imm2_1((imm >> 1) & 0x3);
    inst.set_imm7_6((imm >> 6) & 0x3);
    inst.set_imm4_3((imm >> 3) & 0x3);
    inst.set_imm12(((imm >> 8) & 1) != 0);
    inst.into()
}

/// CJ 型压缩跳转编码。
#[inline]
fn cj_type(funct3: u16, offset: i32) -> u16 {
    debug_assert_eq!(offset & 1, 0, "压缩跳转偏移必须 2 字节对齐");
    let imm = offset as u16;
    let mut inst = CJTypeInstruction::new();
    inst.set_opcode(0b01);
    inst.set_funct3(funct3);
    inst.set_imm5(((imm >> 5) & 1) != 0);
    inst.set_imm3_1((imm >> 1) & 0x7);
    inst.set_imm7(((imm >> 7) & 1) != 0);
    inst.set_imm6(((imm >> 6) & 1) != 0);
    inst.set_imm10(((imm >> 10) & 1) != 0);
    inst.set_imm9_8((imm >> 8) & 0x3);
    inst.set_imm4(((imm >> 4) & 1) != 0);
    inst.set_imm11(((imm >> 11) & 1) != 0);
    inst.into()
}
