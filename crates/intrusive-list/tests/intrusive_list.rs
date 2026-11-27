use goethite_intrusive_list::IntrusiveList;

/// 用 `usize` 数组充当侵入式节点：每个元素的首字保存“下一个节点”指针。
struct NodePool {
    storage: Vec<usize>,
}

impl NodePool {
    fn new(count: usize) -> Self {
        Self {
            storage: vec![0; count],
        }
    }

    fn node(&mut self, index: usize) -> *mut usize {
        self.storage.as_mut_ptr().wrapping_add(index)
    }

    fn value(&self, index: usize) -> usize {
        self.storage[index]
    }
}

#[test]
fn push_pop_is_lifo() {
    let mut pool = NodePool::new(3);
    let nodes: Vec<_> = (0..3).map(|i| pool.node(i)).collect();
    let mut list = IntrusiveList::new();

    assert!(list.is_empty());
    assert_eq!(list.pop(), None);

    list.push(nodes[0]);
    list.push(nodes[1]);
    list.push(nodes[2]);
    assert!(!list.is_empty());

    assert_eq!(list.pop(), Some(nodes[2]));
    assert_eq!(list.pop(), Some(nodes[1]));
    assert_eq!(list.pop(), Some(nodes[0]));
    assert!(list.is_empty());
}

#[test]
fn iter_walks_front_to_back() {
    let mut pool = NodePool::new(4);
    let nodes: Vec<_> = (0..4).map(|i| pool.node(i)).collect();
    let mut list = IntrusiveList::new();

    for &node in &nodes {
        list.push(node);
    }

    let seen: Vec<_> = list.iter().collect();
    assert_eq!(seen, vec![nodes[3], nodes[2], nodes[1], nodes[0]]);

    // `iter_mut` 与 `iter` 遍历顺序一致，且不改变链表状态。
    let seen_mut: Vec<_> = list.iter_mut().collect();
    assert_eq!(seen_mut, seen);
    assert_eq!(list.iter().count(), 4);
}

#[test]
fn remove_head_middle_and_missing() {
    let mut pool = NodePool::new(4);
    let nodes: Vec<_> = (0..4).map(|i| pool.node(i)).collect();
    let mut list = IntrusiveList::new();

    for &node in &nodes {
        list.push(node);
    }

    assert!(list.remove(nodes[3] as usize));
    assert_eq!(
        list.iter().collect::<Vec<_>>(),
        vec![nodes[2], nodes[1], nodes[0]]
    );

    assert!(list.remove(nodes[1] as usize));
    assert_eq!(list.iter().collect::<Vec<_>>(), vec![nodes[2], nodes[0]]);

    assert!(list.remove(nodes[0] as usize));
    assert_eq!(list.iter().collect::<Vec<_>>(), vec![nodes[2]]);

    assert!(!list.remove(nodes[0] as usize));
    assert!(!list.remove(0xdead_beef));
}

#[test]
fn empty_list_debug_and_equality() {
    let mut list = IntrusiveList::new();
    assert_eq!(list, IntrusiveList::new());
    assert_eq!(format!("{list:?}"), "[ ]");

    let mut pool = NodePool::new(1);
    let node = pool.node(0);
    list.push(node);
    assert_ne!(list, IntrusiveList::new());
    assert_eq!(format!("{list:?}"), format!("[ {:#x} ]", node as usize));

    // 节点首字保存了旧头指针（空链表时为 0）。
    assert_eq!(pool.value(0), 0);
}

#[test]
fn default_constructs_empty_list() {
    let list = IntrusiveList::default();
    assert!(list.is_empty());
}
