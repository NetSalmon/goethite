#![no_std]
#![allow(unused)]

//! goethite 内核的系统调用接口雏形。
//!
//! 本 crate 只刻画「即将落地的系统调用接口的形状」，不包含具体实现：
//!
//! - [`PosixErrno`]：以 Linux/POSIX 惯例为蓝本的错误码包装类型。由 `numeric!` 宏生成的
//!   `#[repr(transparent)]` 新类型，底层为 [`i64`]；数值 `0` 表示成功、非 `0`（取负）
//!   表示错误（如 `-EINVAL`、`-ENOENT`）。当前大括号内尚未声明具体常量。
//! - [`Syscall`]：用 Rust 类型安全地描述各底层 syscall 的入参与语义，便于将来
//!   在内核里按 syscall number 分发。
//!

use goethite_macros::numeric;

numeric! {
    pub enum PosixErrno : i64 {
        EPERM = 1,          // Operation not permitted
        ENOENT = 2,         // No such file or directory
        ESRCH = 3,          // No such process
        EINTR = 4,          // Interrupted system call
        EIO = 5,            // Input/output error
        ENXIO = 6,          // No such device or address
        E2BIG = 7,          // Argument list too long
        ENOEXEC = 8,        // Exec format error
        EBADF = 9,          // Bad file descriptor
        ECHILD = 10,        // No child processes
        EAGAIN = 11,        // Resource temporarily unavailable
        EWOULDBLOCK = 11,   // Operation would block (same as EAGAIN)
        ENOMEM = 12,        // Cannot allocate memory
        EACCES = 13,        // Permission denied
        EFAULT = 14,        // Bad address
        ENOTBLK = 15,       // Block device required
        EBUSY = 16,         // Device or resource busy
        EEXIST = 17,        // File exists
        EXDEV = 18,         // Invalid cross-device link
        ENODEV = 19,        // No such device
        ENOTDIR = 20,       // Not a directory
        EISDIR = 21,        // Is a directory
        EINVAL = 22,        // Invalid argument
        ENFILE = 23,        // Too many open files in system
        EMFILE = 24,        // Too many open files
        ENOTTY = 25,        // Inappropriate ioctl for device
        ETXTBSY = 26,       // Text file busy
        EFBIG = 27,         // File too large
        ENOSPC = 28,        // No space left on device
        ESPIPE = 29,        // Illegal seek
        EROFS = 30,         // Read-only file system
        EMLINK = 31,        // Too many links
        EPIPE = 32,         // Broken pipe
        EDOM = 33,          // Numerical argument out of domain
        ERANGE = 34,        // Numerical result out of range
        EDEADLK = 35,       // Resource deadlock avoided
        EDEADLOCK = 35,     // Resource deadlock avoided (same as EDEADLK)
        ENAMETOOLONG = 36,  // File name too long
        ENOLCK = 37,        // No locks available
        ENOSYS = 38,        // Function not implemented
        ENOTEMPTY = 39,     // Directory not empty
        ELOOP = 40,         // Too many levels of symbolic links
        ENOMSG = 42,        // No message of desired type
        EIDRM = 43,         // Identifier removed
        ECHRNG = 44,        // Channel number out of range
        EL2NSYNC = 45,      // Level 2 not synchronized
        EL3HLT = 46,        // Level 3 halted
        EL3RST = 47,        // Level 3 reset
        ELNRNG = 48,        // Link number out of range
        EUNATCH = 49,       // Protocol driver not attached
        ENOCSI = 50,        // No CSI structure available
        EL2HLT = 51,        // Level 2 halted
        EBADE = 52,         // Invalid exchange
        EBADR = 53,         // Invalid request descriptor
        EXFULL = 54,        // Exchange full
        ENOANO = 55,        // No anode
        EBADRQC = 56,       // Invalid request code
        EBADSLT = 57,       // Invalid slot
        EBFONT = 59,        // Bad font file format
        ENOSTR = 60,        // Device not a stream
        ENODATA = 61,       // No data available
        ETIME = 62,         // Timer expired
        ENOSR = 63,         // Out of streams resources
        ENONET = 64,        // Machine is not on the network
        ENOPKG = 65,        // Package not installed
        EREMOTE = 66,       // Object is remote
        ENOLINK = 67,       // Link has been severed
        EADV = 68,          // Advertise error
        ESRMNT = 69,        // Srmount error
        ECOMM = 70,         // Communication error on send
        EPROTO = 71,        // Protocol error
        EMULTIHOP = 72,     // Multihop attempted
        EDOTDOT = 73,       // RFS specific error
        EBADMSG = 74,       // Bad message
        EOVERFLOW = 75,     // Value too large for defined data type
        ENOTUNIQ = 76,      // Name not unique on network
        EBADFD = 77,        // File descriptor in bad state
        EREMCHG = 78,       // Remote address changed
        ELIBACC = 79,       // Can not access a needed shared library
        ELIBBAD = 80,       // Accessing a corrupted shared library
        ELIBSCN = 81,       // .lib section in a.out corrupted
        ELIBMAX = 82,       // Attempting to link in too many shared libraries
        ELIBEXEC = 83,      // Cannot exec a shared library directly
        EILSEQ = 84,        // Invalid or incomplete multibyte or wide character
        ERESTART = 85,      // Interrupted system call should be restarted
        ESTRPIPE = 86,      // Streams pipe error
        EUSERS = 87,        // Too many users
        ENOTSOCK = 88,      // Socket operation on non-socket
        EDESTADDRREQ = 89,  // Destination address required
        EMSGSIZE = 90,      // Message too long
        EPROTOTYPE = 91,    // Protocol wrong type for socket
        ENOPROTOOPT = 92,   // Protocol not available
        EPROTONOSUPPORT = 93,// Protocol not supported
        ESOCKTNOSUPPORT = 94,// Socket type not supported
        EOPNOTSUPP = 95,    // Operation not supported on transport endpoint
        ENOTSUP = 95,       // Operation not supported
        EPFNOSUPPORT = 96,  // Protocol family not supported
        EAFNOSUPPORT = 97,  // Address family not supported by protocol
        EADDRINUSE = 98,    // Address already in use
        EADDRNOTAVAIL = 99, // Cannot assign requested address
        ENETDOWN = 100,     // Network is down
        ENETUNREACH = 101,  // Network is unreachable
        ENETRESET = 102,    // Network dropped connection on reset
        ECONNABORTED = 103, // Software caused connection abort
        ECONNRESET = 104,   // Connection reset by peer
        ENOBUFS = 105,      // No buffer space available
        EISCONN = 106,      // Transport endpoint is already connected
        ENOTCONN = 107,     // Transport endpoint is not connected
        ESHUTDOWN = 108,    // Cannot send after transport endpoint shutdown
        ETOOMANYREFS = 109, // Too many references: cannot splice
        ETIMEDOUT = 110,    // Connection timed out
        ECONNREFUSED = 111, // Connection refused
        EHOSTDOWN = 112,    // Host is down
        EHOSTUNREACH = 113, // No route to host
        EALREADY = 114,     // Operation already in progress
        EINPROGRESS = 115,  // Operation now in progress
        ESTALE = 116,       // Stale file handle
        EUCLEAN = 117,      // Structure needs cleaning
        ENOTNAM = 118,      // Not a XENIX named type file
        ENAVAIL = 119,      // No XENIX semaphores available
        EISNAM = 120,       // Is a named type file
        EREMOTEIO = 121,    // Remote I/O error
        EDQUOT = 122,       // Disk quota exceeded
        ENOMEDIUM = 123,    // No medium found
        EMEDIUMTYPE = 124,  // Wrong medium type
        ECANCELED = 125,    // Operation canceled
        ENOKEY = 126,       // Required key not available
        EKEYEXPIRED = 127,  // Key has expired
        EKEYREVOKED = 128,  // Key has been revoked
        EKEYREJECTED = 129, // Key was rejected by service
        EOWNERDEAD = 130,   // Owner died
        ENOTRECOVERABLE = 131,// State not recoverable
        ERFKILL = 132,      // Operation not possible due to RF-kill
        EHWPOISON = 133,    // Memory page has hardware error
    }
}

pub type SysResult<T> = Result<T, PosixErrno>;
pub type Fd = usize;

/// 系统调用接口的形状。
///
/// 用 Rust 的类型系统把各个底层 syscall 的参数固化下来，方便将来在内核里
/// 按 syscall number 分发到具体实现。各方法的语义按 Linux/POSIX 惯例描述，
/// 这只是接口约定，不代表内核已经实现了这些系统调用；方法签名也未预先定死
/// 返回值，由实现层决定如何表达结果。
pub trait SyscallOps {
    /// 打开一个文件（Linux `open`）。
    ///
    /// 按惯例接受文件路径与打开标志，成功返回文件描述符（fd），失败返回错误码。
    /// 当前签名尚未携带这些参数，仅作为接口形状占位。
    fn open() -> SysResult<Fd> {
        Err(PosixErrno::ENOSYS)
    }

    /// 关闭文件描述符 `fd`（Linux `close`）。
    ///
    /// 释放 `fd` 对应打开文件资源；后续对同一 `fd` 的访问应失败。返回 `0` 表示
    /// 成功，负错误码表示失败。
    fn close(fd: usize) -> SysResult<()> {
        Err(PosixErrno::ENOSYS)
    }

    /// 从 `fd` 读取数据到 `buf`（Linux `read`）。
    ///
    /// 最多读 `buf` 长度字节；返回实际读到的字节数，读到文件尾返回 `0`，
    /// 出错返回负错误码。是否有数据可读由文件当前位置等因素决定。
    fn read(fd: usize, buf: &mut [u8]) -> SysResult<usize> {
        Err(PosixErrno::ENOSYS)
    }

    /// 向 `fd` 写入 `buf`（Linux `write`）。
    ///
    /// 按惯例返回实际写入的字节数，出错返回负错误码；写入多少、是否成功由
    /// 实现层决定。
    fn write(fd: usize, buf: &[u8]) -> SysResult<usize> {
        Err(PosixErrno::ENOSYS)
    }

    /// 获取已打开文件 `fd` 的状态（Linux `stat`）。
    ///
    /// 读取文件元数据（大小、权限、类型等）。按惯例需要一个用于写回结果的
    /// `stat` 结构体指针，当前签名尚未携带该参数，属接口形状占位。
    fn stat(fd: usize) -> SysResult<i32> {
        Err(PosixErrno::ENOSYS)
    }

    /// 通过路径 `path` 获取文件状态（Linux `stat` / `lstat`）。
    ///
    /// 按路径名定位文件并读取其元数据（大小、权限、类型等），按惯例将结果写入
    /// 传入的 `stat` 结构体（当前签名尚未携带该参数）；路径不存在时按惯例返回
    /// `ENOENT` 类错误码。
    fn file_stat(path: &str) -> SysResult<i32> {
        Err(PosixErrno::ENOSYS)
    }

    /// 创建子进程（Linux `fork`）。
    ///
    /// 复制当前进程为子进程：父进程返回子进程 pid，子进程返回 `0`，出错返回
    /// 负错误码。
    fn fork() -> SysResult<usize> {
        Err(PosixErrno::ENOSYS)
    }

    /// 用新程序替换当前进程映像（Linux `execve`）。
    ///
    /// 加载 `path` 指向的可执行文件，以 `args` 作为 argv、`env` 作为环境变量
    /// 启动；成功后不返回（进程映像已被替换），失败返回负错误码。
    fn execve(path: &str, args: &[&str], env: &[&str]) -> SysResult<i32> {
        Err(PosixErrno::ENOSYS)
    }

    fn anonymous_map(vaddr: usize, paddr: usize, size: usize, flags: usize) -> SysResult<usize> {
        Err(PosixErrno::ENOSYS)
    }

    fn file_map(
        vaddr: usize,
        paddr: usize,
        size: usize,
        flags: usize,
        file: &str,
        offset: usize,
    ) -> SysResult<usize> {
        Err(PosixErrno::ENOSYS)
    }
}

numeric! {
    pub enum Syscall : u64 {
        OPEN = 0,
        CLOSE = 1,
        READ = 2,
        WRITE = 3,
        STAT = 4,
        FILE_STAT = 5,
        FORK = 6,
        EXECVE = 7,
    }
}
