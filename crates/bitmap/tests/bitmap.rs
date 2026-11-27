use goethite_bitmap::{Bitmap, BitmapError, BitmapView, words_for};

#[test]
fn alloc_and_free_roundtrip() {
    let mut bitmap = Bitmap::<2>::new_with_capacity(100);
    assert_eq!(bitmap.bits(), 100);
    assert!(bitmap.is_empty());
    assert_eq!(bitmap.used(), 0);

    assert_eq!(bitmap.alloc(), Some(0));
    assert_eq!(bitmap.alloc(), Some(1));
    assert!(bitmap.is_allocated(0));
    assert!(bitmap.is_allocated(1));
    assert_eq!(bitmap.used(), 2);
    assert_eq!(bitmap.available(), 98);

    bitmap.free(0).unwrap();
    assert!(bitmap.is_free(0));
    assert_eq!(bitmap.alloc(), Some(0));
    assert_eq!(bitmap.used(), 2);
}

#[test]
fn alloc_specific_and_errors() {
    let mut bitmap = Bitmap::<1>::new_with_capacity(10);

    bitmap.alloc_specific(7).unwrap();
    assert_eq!(bitmap.alloc_specific(7), Err(BitmapError::AlreadySet));
    assert_eq!(bitmap.free(7), Ok(()));
    assert_eq!(bitmap.free(7), Err(BitmapError::AlreadyFree));
    assert_eq!(bitmap.get(10), Err(BitmapError::OutOfRange));
    assert_eq!(bitmap.set(10), Err(BitmapError::OutOfRange));
    assert_eq!(bitmap.alloc_specific(10), Err(BitmapError::OutOfRange));
}

#[test]
fn range_allocation() {
    let mut bitmap = Bitmap::<2>::new_with_capacity(64);
    assert_eq!(bitmap.alloc_range(4), Some(0));
    assert_eq!(bitmap.alloc_range(3), Some(4));
    assert_eq!(bitmap.alloc_range(0), None);

    assert_eq!(bitmap.free_range(0..4), Ok(()));
    assert_eq!(bitmap.alloc_range(4), Some(0));
    assert_eq!(bitmap.free_range(0..2), Ok(()));
    assert_eq!(bitmap.free_range(0..2), Err(BitmapError::AlreadyFree));
    assert_eq!(bitmap.free_range(0..100), Err(BitmapError::OutOfRange));
}

#[test]
fn iterators_and_find() {
    let mut bitmap = Bitmap::<1>::new_with_capacity(16);
    for index in [1, 4, 5, 15] {
        bitmap.set(index).unwrap();
    }

    assert_eq!(bitmap.iter_used().collect::<Vec<_>>(), vec![1, 4, 5, 15]);
    assert_eq!(
        bitmap.iter_free().collect::<Vec<_>>(),
        vec![0, 2, 3, 6, 7, 8, 9, 10, 11, 12, 13, 14]
    );
    assert_eq!(bitmap.find_free(), Some(0));
    assert_eq!(bitmap.find_free_from(4), Some(6));
    assert_eq!(bitmap.find_used(), Some(1));
    assert_eq!(bitmap.find_used_from(2), Some(4));
    assert_eq!(bitmap.find_used_from(16), None);
}

#[test]
fn conversions() {
    let bitmap = Bitmap::<1>::from([0b1011usize]);
    assert_eq!(bitmap.bits(), usize::BITS as usize);
    assert!(bitmap.is_allocated(0));
    assert!(bitmap.is_allocated(1));
    assert!(!bitmap.is_allocated(2));
    assert!(bitmap.is_allocated(3));
    assert_eq!(bitmap.used(), 3);

    let byte = 1usize.to_le_bytes();
    let bitmap = Bitmap::<1>::try_from_le_bytes(&byte).unwrap();
    assert!(bitmap.is_allocated(0));
    assert_eq!(bitmap.used(), 1);

    let byte = 1usize.to_be_bytes();
    let bitmap = Bitmap::<1>::try_from_be_bytes(&byte).unwrap();
    assert!(bitmap.is_allocated(0));

    let msb = 1usize << (usize::BITS as usize - 1);
    let bitmap = Bitmap::<1>::try_from_be_bytes(&msb.to_be_bytes()).unwrap();
    assert!(bitmap.is_allocated(usize::BITS as usize - 1));

    assert_eq!(
        Bitmap::<1>::try_from_le_bytes(&[0u8; 3]),
        Err(BitmapError::LengthMismatch)
    );
}

#[test]
fn zero_copy_view() {
    let mut words = [0usize; 2];
    {
        let mut view = BitmapView::new_with_capacity(&mut words, 70);
        assert_eq!(view.bits(), 70);
        assert_eq!(view.alloc_range(3), Some(0));
        view.alloc_specific(69).unwrap();
        assert_eq!(view.used(), 4);
        assert_eq!(view.iter_used().collect::<Vec<_>>(), vec![0, 1, 2, 69]);
    }
    assert_eq!(words[0] & 0b111, 0b111);
}

#[test]
fn view_from_bytes_checks_alignment_and_length() {
    #[repr(align(8))]
    struct Aligned([u8; 16]);

    let mut aligned = Aligned([0; 16]);
    {
        let mut view = BitmapView::try_from(&mut aligned.0[..]).unwrap();
        assert_eq!(view.bits(), 128);
        view.alloc().unwrap();
    }
    assert_eq!(aligned.0[0] & 1, 1);

    let mut odd = [0u8; 3];
    assert!(matches!(
        BitmapView::try_from(&mut odd[..]),
        Err(BitmapError::LengthMismatch)
    ));
}

#[test]
fn words_for_rounds_up() {
    let bits = usize::BITS as usize;
    assert_eq!(words_for(0), 0);
    assert_eq!(words_for(1), 1);
    assert_eq!(words_for(bits), 1);
    assert_eq!(words_for(bits + 1), 2);
}
