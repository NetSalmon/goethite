use goethite_id_alloc::{Id, IdAlloc, IdAllocError};

#[derive(goethite_id_alloc::Id)]
struct SmallId(pub u16);

#[test]
fn plain_allocation_and_deallocation() {
    let mut alloc = IdAlloc::<u16, 0>::from_range(10..14).unwrap();
    assert_eq!(alloc.size(), 4);
    assert_eq!(alloc.available(), 4);

    assert_eq!(alloc.alloc(), Some(10));
    assert_eq!(alloc.alloc(), Some(11));
    assert_eq!(alloc.alloc_at(13), Ok(()));
    assert!(alloc.is_allocated(13));
    assert_eq!(alloc.used(), 3);
    assert_eq!(alloc.alloc(), Some(12));
    assert_eq!(alloc.alloc(), None);
    assert!(alloc.is_full());

    assert_eq!(alloc.dealloc(13), Ok(()));
    assert_eq!(alloc.dealloc(13), Err(IdAllocError::AlreadyFree));
    assert_eq!(alloc.dealloc(99), Err(IdAllocError::OutOfRange));
    assert_eq!(alloc.alloc(), Some(13));
}

#[test]
fn major_minor_routing() {
    let mut alloc = IdAlloc::<u16, 4>::from_range(0..64).unwrap();

    let major = alloc.alloc_major().unwrap();
    assert_eq!(major, 0);
    assert_eq!(alloc.available(), 64);
    assert!(alloc.is_empty());

    assert_eq!(alloc.alloc_minor(major), Some(0));
    assert_eq!(alloc.alloc_minor(major), Some(1));
    assert!(alloc.is_allocated(0));
    assert!(alloc.is_allocated(1));
    assert_eq!(alloc.used(), 2);

    // 扁平路径不能进入已预留的 major 块。
    assert_eq!(alloc.alloc(), Some(16));

    assert_eq!(alloc.dealloc(0), Ok(()));
    assert_eq!(alloc.alloc_minor(major), Some(0));

    assert_eq!(alloc.dealloc_major(major), Ok(()));
    assert_eq!(alloc.dealloc_major(major), Err(IdAllocError::MajorNotFound));
    assert_eq!(alloc.alloc(), Some(0));
}

#[test]
fn major_at_and_alignment_errors() {
    let mut alloc = IdAlloc::<u16, 4>::from_range(16..96).unwrap();
    assert!(matches!(
        IdAlloc::<u16, 4>::from_range(1..16),
        Err(IdAllocError::NotAligned)
    ));

    assert_eq!(alloc.alloc_major_at(16), Ok(16));
    assert_eq!(
        alloc.alloc_major_at(16),
        Err(IdAllocError::AlreadyAllocated)
    );
    assert_eq!(alloc.alloc_major_at(17), Err(IdAllocError::NotAligned));
    assert_eq!(alloc.alloc_major_at(80), Ok(80));
    assert_eq!(alloc.alloc_minor(16), Some(16));
}

#[test]
fn reset_restores_capacity() {
    let mut alloc = IdAlloc::<u8, 0>::from_range(0..8).unwrap();
    assert_eq!(alloc.alloc(), Some(0));
    assert_eq!(alloc.alloc_at(4), Ok(()));
    alloc.reset();
    assert!(alloc.is_empty());
    assert_eq!(alloc.available(), 8);
    assert_eq!(alloc.alloc(), Some(0));
}

#[test]
fn custom_id_derive() {
    let mut alloc = IdAlloc::<SmallId, 0>::from_range(SmallId(3)..SmallId(5)).unwrap();
    assert_eq!(alloc.alloc(), Some(SmallId(3)));
    assert_eq!(alloc.alloc(), Some(SmallId(4)));
    assert_eq!(alloc.alloc(), None);
    assert_eq!(SmallId(9).to_bits(), 9);
    assert_eq!(SmallId::from_bits(7), SmallId(7));
    assert_eq!(SmallId::MIN, SmallId(0));
}
