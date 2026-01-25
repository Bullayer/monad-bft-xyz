use rayon::iter::{Either, IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use futures::{stream, StreamExt};

#[test]
fn test_rayon() {
    let thread_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(10)
        .thread_name(|idx| format!("test-thread-{}", idx))
        .build().unwrap_or_else(|e| panic!("bad rayon -- {}", e));

    let data: Vec<_> = (1..100).collect();

    // 使用自定义线程池进行并行计算
    let sum: i32 = thread_pool.install(|| {
        data.par_iter().map(|d| {
            println!("Processing {} in thread: {:?}", d, std::thread::current().name());
            d * d
        }).sum()
    });
    println!("the len is {}", data.len());
    println!("the sum is {}", sum);

    // 在自定义线程池中运行partition_map
    let data: Vec<_> = (0..100).collect();
    let (a, b): (Vec<_>, Vec<_>) = thread_pool.install(|| {
        data.into_par_iter().partition_map(|d| {
            if d < 50 {
                Either::Left(d)
            } else {
                Either::Right(d)
            }
        })
    });
    assert_eq!(a.len(), b.len());
}

// Future 示例：async函数返回Future
async fn f1() -> i32 {
    println!("f1 start");
    let y = f2().await;  // await会等待Future完成
    321 + y
}

// 另一个async函数
async fn f2() -> i32 {
    println!("f2 start");
    std::thread::sleep(std::time::Duration::from_secs(3));
    123  // 同步返回
}

// 使用Future和Stream的完整示例
async fn demo_future_and_stream() {
    // Future: 延迟计算的值
    let future_value = async {
        println!("Future开始计算");
        f1().await
    };

    // Stream: 值序列
    let stream = stream::iter(vec![1, 2, 3, 4, 5]);

    // 并发处理Future和Stream
    let (future_result, stream_sum) = tokio::join!(
        future_value,  // 执行Future
        async {
            let mut sum = 0;
            let mut stream = stream;
            while let Some(item) = stream.next().await {
                sum += item;
            }
            println!("Stream完成计算");
            sum
        }
    );

    println!("Future结果: {}, Stream求和: {}", future_result, stream_sum);
}

#[test]
fn test_tokio_manual_runtime() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        demo_future_and_stream().await;
        println!("手动runtime测试完成");
    });
}

#[test]
fn test_ping_pong() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    
    rt.block_on(async {
        tokio::spawn(async {
            loop {
                println!("tick");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
    
        tokio::spawn(async {
            loop {
                println!("tock");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
    });

    std::thread::sleep(std::time::Duration::from_secs(3));
}

#[test]
fn test_option() {
    let mut op = Some(9);
    assert!(op.is_some());
    assert_eq!(op.is_none(), false);
    assert!(op.is_some_and(|s| s > 8));
    assert_eq!(op.is_none_or(|s| s > 10), false);

    // Converts from `&Option<T>` to `Option<&T>`.
    assert_eq!(op.as_ref(), Some(&9));
    // Converts from `&mut Option<T>` to `Option<&mut T>`.
    match op.as_mut() {
        Some(v) => *v = *v * *v,
        None => {},
    }
    assert_eq!(op, Some(81));

    assert_eq!(op.unwrap(), 81);
    assert_eq!(None.unwrap_or_else(|| 666), 666);

    assert_eq!(op.map(|s| s + 100), Some(181));
    assert_eq!(None.map_or(777, |x| x), 777);

    assert_eq!(op.into_iter().next(), Some(81));
    assert_eq!(op.iter().next(), Some(&81));
}