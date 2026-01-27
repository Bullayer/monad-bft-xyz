use std::collections::VecDeque;

#[test]
fn vec_deque_test() {

    // init
    let deque: VecDeque<u32> = VecDeque::new();

    // len
    assert_eq!(deque.len(), 0);
    // is_empty
    assert_eq!(deque.is_empty(), true);

    let _deque: VecDeque<u32> = VecDeque::with_capacity(10);

    // get
    let mut buf = VecDeque::new();
    buf.push_back(3);
    buf.push_back(4);
    buf.push_back(5);
    buf.push_back(6);
    assert_eq!(buf.get(1), Some(&4));

    // get_mut
    let mut buf = VecDeque::new();
    buf.push_back(3);
    buf.push_back(4);
    buf.push_back(5);
    buf.push_back(6);
    assert_eq!(buf[1], 4);
    if let Some(elem) = buf.get_mut(1) {
        *elem = 7;
    }
    assert_eq!(buf[1], 7);

    // swap
    let mut buf = VecDeque::new();
    buf.push_back(3);
    buf.push_back(4);
    buf.push_back(5);
    assert_eq!(buf, [3, 4, 5]);
    buf.swap(0, 2);
    assert_eq!(buf, [5, 4, 3]);

    //  capacity
    let buf: VecDeque<i32> = VecDeque::with_capacity(10);
    assert!(buf.capacity() >= 10);

    // iter
    let mut buf = VecDeque::new();
    buf.push_back(5);
    buf.push_back(3);
    buf.push_back(4);
    let b: &[_] = &[&5, &3, &4];
    let c: Vec<&i32> = buf.iter().collect();
    assert_eq!(&c[..], b);

    // iter_mut
    let mut buf = VecDeque::new();
    buf.push_back(5);
    buf.push_back(3);
    buf.push_back(4);
    assert_eq!(buf.len(), 3);
    for num in buf.iter_mut() {
        *num = *num - 2;
    }
    let b: &[_] = &[&mut 3, &mut 1, &mut 2];
    assert_eq!(&buf.iter_mut().collect::<Vec<&mut i32>>()[..], b);

    // front
    let mut d = VecDeque::new();
    assert_eq!(d.front(), None);

    d.push_back(1);
    d.push_back(2);
    assert_eq!(d.front(), Some(&1));

    // front_mut
    let mut d = VecDeque::new();
    assert_eq!(d.front_mut(), None);

    d.push_back(1);
    d.push_back(2);
    match d.front_mut() {
        Some(x) => *x = 9,
        None => (),
    }
    assert_eq!(d.front(), Some(&9));

    // back
    let mut d = VecDeque::new();
    assert_eq!(d.back(), None);

    d.push_back(1);
    d.push_back(2);
    assert_eq!(d.back(), Some(&2));

    // back_mut
    let mut d = VecDeque::new();
    assert_eq!(d.back(), None);

    d.push_back(1);
    d.push_back(2);
    match d.back_mut() {
        Some(x) => *x = 9,
        None => (),
    }
    assert_eq!(d.back(), Some(&9));

    // pop_front
    let mut d = VecDeque::new();
    d.push_back(1);
    d.push_back(2);

    assert_eq!(d.pop_front(), Some(1));
    assert_eq!(d.pop_front(), Some(2));
    assert_eq!(d.pop_front(), None);

    // pop_back
    let mut buf = VecDeque::new();
    assert_eq!(buf.pop_back(), None);
    buf.push_back(1);
    buf.push_back(3);
    assert_eq!(buf.pop_back(), Some(3));

    let mut deque: VecDeque<i32> = vec![0, 1, 2, 3, 4].into();
    let pred = |x: &mut i32| *x % 2 == 0;

    // push_front
    let mut d = VecDeque::new();
    d.push_front(1);
    d.push_front(2);
    assert_eq!(d.front(), Some(&2));

    // push_back
    let mut buf = VecDeque::new();
    buf.push_back(1);
    buf.push_back(3);
    assert_eq!(3, *buf.back().unwrap());

    // append
    let mut buf: VecDeque<_> = [1, 2].into();
    let mut buf2: VecDeque<_> = [3, 4].into();
    buf.append(&mut buf2);
    assert_eq!(buf, [1, 2, 3, 4]);
    assert_eq!(buf2, []);
}