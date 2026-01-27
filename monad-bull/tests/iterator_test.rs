use std::iter::zip;

#[test]
fn iterator_test() {

    // next
    let a = [1, 2, 3];
    let mut iter = a.iter();
    // A call to next() returns the next value...
    assert_eq!(Some(&1), iter.next());
    assert_eq!(Some(&2), iter.next());
    assert_eq!(Some(&3), iter.next());
    // ... and then None once it's over.
    assert_eq!(None, iter.next());
    // More calls may or may not return `None`. Here, they always will.
    assert_eq!(None, iter.next());
    assert_eq!(None, iter.next());

    // map
    let a = [1, 2, 3];
    let mut iter = a.iter().map(|x| 2 * x);
    assert_eq!(iter.next(), Some(2));
    assert_eq!(iter.next(), Some(4));
    assert_eq!(iter.next(), Some(6));
    assert_eq!(iter.next(), None);
    // don't do this:
    // (0..5).map(|x| println!("{x}"));
    // Instead, use for:
    for x in 0..5 {
        println!("{x}");
    }

    // filter
    let a = [0i32, 1, 2];
    let mut iter = a.iter().filter(|x| x.is_positive());
    assert_eq!(iter.next(), Some(&1));
    assert_eq!(iter.next(), Some(&2));
    assert_eq!(iter.next(), None);

    // fold
    let a = [1, 2, 3];
    // the sum of all of the elements of the array
    let sum = a.iter().fold(0, |acc, x| acc + x);
    assert_eq!(sum, 6);

    let numbers = [1, 2, 3, 4, 5];
    let zero = "0".to_string();
    let result = numbers.iter().fold(zero, |acc, &x| {
        format!("({acc} + {x})")
    });
    assert_eq!(result, "(((((0 + 1) + 2) + 3) + 4) + 5)"); // because of the left-associative nature

    let numbers = [1, 2, 3, 4, 5];
    let mut result = 0;
    // for loop:
    for i in &numbers {
        result = result + i;
    }
    // fold:
    let result2 = numbers.iter().fold(0, |acc, &x| acc + x);
    // they're the same
    assert_eq!(result, result2);

    // collect
    let a = [1, 2, 3];
    let doubled: Vec<i32> = a.iter()
        .map(|&x| x * 2)
        .collect();
    assert_eq!(vec![2, 4, 6], doubled);

    let a = [1, 2, 3];
    let doubled: Vec<i32> = a.iter()
        .map(|&x| x * 2)
        .collect();
    assert_eq!(2, doubled[0]);
    assert_eq!(4, doubled[1]);
    assert_eq!(6, doubled[2]);

    let a = [1, 2, 3];
    let doubled = a.iter().map(|&x| x * 2).collect::<Vec<i32>>();
    assert_eq!(vec![2, 4, 6], doubled);

    let chars = ['g', 'd', 'k', 'k', 'n'];
    let hello: String = chars.iter()
        .map(|&x| x as u8)
        .map(|x| (x + 1) as char)
        .collect();
    assert_eq!("hello", hello);

    let results = [Ok(1), Err("nope"), Ok(3), Err("bad")];
    let result: Result<Vec<_>, &str> = results.iter().cloned().collect();
    // gives us the first error - 短路行为 (Short-circuiting)
    assert_eq!(Err("nope"), result);
    let results = [Ok(1), Ok(3)];
    let result: Result<Vec<_>, &str> = results.iter().cloned().collect();
    // gives us the list of answers
    assert_eq!(Ok(vec![1, 3]), result);

    // zip
    let a1 = [1, 2, 3];
    let a2 = [4, 5, 6];
    let mut iter = a1.iter().zip(a2.iter());
    assert_eq!(iter.next(), Some((&1, &4)));
    assert_eq!(iter.next(), Some((&2, &5)));
    assert_eq!(iter.next(), Some((&3, &6)));
    assert_eq!(iter.next(), None);

    let s1 = &[1, 2, 3];
    let s2 = &[4, 5, 6];
    let mut iter = s1.iter().zip(s2);
    assert_eq!(iter.next(), Some((&1, &4)));
    assert_eq!(iter.next(), Some((&2, &5)));
    assert_eq!(iter.next(), Some((&3, &6)));
    assert_eq!(iter.next(), None);

    // zip() is often used to zip an infinite iterator to a finite one.
    // This works because the finite iterator will eventually return None, ending the zipper.
    // Zipping with (0..) can look a lot like enumerate:
    let enumerate: Vec<_> = "foo".chars().enumerate().collect();
    let zipper: Vec<_> = (0..).zip("foo".chars()).collect();
    assert_eq!((0, 'f'), enumerate[0]);
    assert_eq!((0, 'f'), zipper[0]);
    assert_eq!((1, 'o'), enumerate[1]);
    assert_eq!((1, 'o'), zipper[1]);
    assert_eq!((2, 'o'), enumerate[2]);
    assert_eq!((2, 'o'), zipper[2]);

    // If both iterators have roughly equivalent syntax, it may be more readable to use zip:
    let a = [1, 2, 3];
    let b = [2, 3, 4];
    let mut zipped = zip(
        a.into_iter().map(|x| x * 2).skip(1),
        b.into_iter().map(|x| x * 2).skip(1),
    );
    assert_eq!(zipped.next(), Some((4, 6)));
    assert_eq!(zipped.next(), Some((6, 8)));
    assert_eq!(zipped.next(), None);
    // compared to:
    let mut zipped = a
        .into_iter()
        .map(|x| x * 2)
        .skip(1)
        .zip(b.into_iter().map(|x| x * 2).skip(1));
    assert_eq!(zipped.next(), Some((4, 6)));
    assert_eq!(zipped.next(), Some((6, 8)));
    assert_eq!(zipped.next(), None);

    // take
    let a = [1, 2, 3];
    let mut iter = a.iter().take(2);
    assert_eq!(iter.next(), Some(&1));
    assert_eq!(iter.next(), Some(&2));
    assert_eq!(iter.next(), None);
    // take() is often used with an infinite iterator, to make it finite:
    let mut iter = (0..).take(3);
    assert_eq!(iter.next(), Some(0));
    assert_eq!(iter.next(), Some(1));
    assert_eq!(iter.next(), Some(2));
    assert_eq!(iter.next(), None);
    // If less than n elements are available, take will limit itself to the size of the underlying iterator:
    let v = [1, 2];
    let mut iter = v.into_iter().take(5);
    assert_eq!(iter.next(), Some(1));
    assert_eq!(iter.next(), Some(2));
    assert_eq!(iter.next(), None);
}