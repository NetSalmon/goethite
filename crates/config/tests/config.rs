use goethite_config::*;

#[test]
fn page_and_alignment_helpers() {
    assert_eq!(PAGE_SIZE, 4096);
    assert_eq!(PAGE_SIZE_LOG_2, 12);
    assert_eq!(ilog2_ceil(1), 0);
    assert_eq!(ilog2_ceil(2), 1);
    assert_eq!(ilog2_ceil(3), 2);
    assert_eq!(ilog2_ceil(4095), 12);
    assert_eq!(ilog2_ceil(4096), 12);
    assert_eq!(ilog2_ceil(4097), 13);

    assert_eq!(align(0, 4096), 0);
    assert_eq!(align(1, 4096), 4096);
    assert_eq!(align(4096, 4096), 4096);
    assert_eq!(align(4097, 4096), 8192);
    assert_eq!(align(0x1234, 16), 0x1240);
}

#[test]
fn memory_layout_constants() {
    assert_eq!(BUDDY_MAX_ORDER, 11);
    assert_eq!(CACHES_MAX, SLUB_MAX_ORDER - SLUB_MIN_ORDER);
    assert_eq!(CACHES_MAX, 8);
    assert_eq!(MAX_KERNEL_HEAP_SIZE, 20 * 1024 * 1024);
    assert_eq!(SLUB_MAX_PAGE_SIZE, 1 << 20);
    assert_eq!(IDENTITY_MAP_CHUNK, 8 * 1024 * 1024);
    assert_eq!(KERNEL_OFFSET, 0xffff_ffc0_0000_0000);
    assert_eq!(USER_STACK_SIZE, 32 * 1024);
    assert_eq!(USER_STACK_TOP, 0x1_0000_0000);
    assert_eq!(USER_STACK_LOWER_BOUND, USER_STACK_TOP - USER_STACK_SIZE);
    assert_eq!(USER_STACK_SIZE % PAGE_SIZE, 0);
    assert_eq!(USER_STACK_TOP % PAGE_SIZE, 0);
    assert_eq!(PTE_NUMBER, 512);
    assert_eq!(PAGE_TABLE_SIZE, 512);
}

#[test]
fn vpn2_extracts_top_index() {
    assert_eq!(vpn2(0), 0);
    assert_eq!(vpn2(0x0000_0000_4000_0000), 1);
    assert_eq!(vpn2(0x0000_00ff_c000_0000), 511);
    assert_eq!(vpn2(KERNEL_OFFSET), KERNEL_PTE_START);
    assert_eq!(KERNEL_PTE_START, 256);
}

#[test]
fn cpu_and_task_limits() {
    assert_eq!(MAX_CPU, 1);
    assert_eq!(TID_MAX, 1024);
    assert_eq!(goethite_config::proc::INIT_PATH[0], "/sbin/init");
    assert!(goethite_config::proc::INIT_PATH.contains(&"/init"));
}

#[test]
fn device_id_space() {
    assert_eq!(DEVICE_ID_MINOR_BITS, 8);
    assert_eq!(DEVICE_ID_MAJOR_MAX, 16);
    assert_eq!(DEVICE_ID_MAX, 16 << 8);
    assert_eq!(DEVICE_ID_MAX, 4096);
}

#[test]
fn uname_constants() {
    assert_eq!(SYS, "Chaahann");
    assert_eq!(RELEASE, "none");
    assert_eq!(ARCH, "riscv64gc");
    assert_eq!(VERSION, "0.1.0");
}

#[test]
fn filesystem_and_virtio_constants() {
    assert_eq!(PATH_SEPARATOR, "/");
    assert_eq!(S_IFMT, 0o170000);
    assert_eq!(SUPERBLOCK_OFFSET, 1024);
    assert_eq!(MAX_SYMLINK_HOPS, 40);
    assert_eq!(MINIX_DIRECT_ZONES, 7);

    assert_eq!(SECTOR_SIZE, 512);
    assert_eq!(MAGIC_VALUE, 0x7472_6976);
    assert_eq!(VIRTIO_VERSION_LEGACY, 1);
    assert_eq!(RING_SIZE, 256);
    assert_eq!(RING_SIZE % 2, 0);
    assert_eq!(VIRTIO_BLK_T_IN, 0);
    assert_eq!(VIRTIO_BLK_T_OUT, 1);
    assert_eq!(VIRTIO_BLK_S_OK, 0);
    assert_eq!(VIRTIO_NET_HDR_LEGACY, 10);
    assert_eq!(VIRTIO_NET_HDR_MODERN, 12);
    assert_eq!(VIRTIO_F_VERSION_1, 1);
    assert_eq!(VIRTQ_AVAIL_F_NO_INTERRUPT, 1);
}
