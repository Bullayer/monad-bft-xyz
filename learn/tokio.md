# Tokio 学习笔记

## 什么是 Tokio？

Tokio 是 Rust 的**异步运行时**，相当于 Java 的：
- Netty（网络框架）
- CompletableFuture（异步任务）
- Virtual Threads（协程）
- 线程池（任务调度）

```
一句话：Tokio = 异步 I/O + 事件循环 + 任务调度
```

---

## 核心概念

### 1. 异步任务 (Task)

```rust
// 顺序执行：-blocking-
fn main() {
    let response = blocking_http_call();  // 线程阻塞等待
    println!("{}", response);
}

// 异步执行：non-blocking
#[tokio::main]
async fn main() {
    let response = http_call().await;  // 切换到其他任务
    println!("{}", response);
}
```

**关键**：`async` 标记可挂起的函数，`.await` 等待结果但不阻塞线程。

---

### 2. Runtime（运行时）

```rust
// 多线程运行时（推荐生产环境）
#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // 所有异步操作都使用这个运行时
}

// 单线程运行时（适合开发调试）
#[tokio::main(flavor = "current_thread")]
async fn main() {
    // 只有一个线程
}
```

**对比**：

| 运行时 | 特点 | 适用场景 |
|--------|------|----------|
| multi_thread | 多线程，充分利用多核 | 生产环境、高并发 |
| current_thread | 单线程，开销小 | 开发调试、轻量应用 |

---

### 3. Spawn（生成任务）

```rust
#[tokio::main]
async fn main() {
    //  spawn 生成异步任务
    tokio::spawn(async {
        "我在后台运行".await;
    });

    // 等待所有任务完成
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
}
```

---

### 4. Join（并发等待）

```rust
// 并发执行多个任务
async fn fetch_all() -> Vec<String> {
    let fut1 = fetch("https://api.a.com");
    let fut2 = fetch("https://api.b.com");
    let fut3 = fetch("https://api.c.com");

    // 等待所有任务完成
    let (r1, r2, r3) = tokio::join!(fut1, fut2, fut3);
    
    vec![r1.unwrap(), r2.unwrap(), r3.unwrap()]
}
```

**vs spawn + join_all**：

```rust
// spawn + join_all：任务之间独立
let handles: Vec<_> = (0..10)
    .map(|i| tokio::spawn(async move { process(i) }))
    .collect();

let results: Vec<_> = futures::future::join_all(handles)
    .await
    .into_iter()
    .collect();
```

---

## Tokio 实现原理

### 1. 整体架构

```txt
┌─────────────────────────────────────────────────────────────────┐
│                         应用程序代码                             │
│         async fn main() { ... spawn(...).await ... }            │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                      异步任务 (Task)                             │
│   每个 async 函数被编译成一个状态机 (Future)                      │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     调度器 (Scheduler)                           │
│   负责决定哪个任务在哪个时刻运行                                  │
│   ├── 多个工作线程 (worker threads)                              │
│   ├── 任务队列 (task queue)                                      │
│   └── 负载均衡                                                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Reactor (反应器)                               │
│   监控 I/O 事件 (epoll/kqueue/IOCP)                             │
│   └── 事件驱动                                                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                   操作系统 (OS Kernel)                           │
│   ├── Linux: epoll                                               │
│   ├── macOS: kqueue                                              │
│   └── Windows: IOCP                                              │
└─────────────────────────────────────────────────────────────────┘
```

---

### 2. Future 状态机

`async` 函数会被编译器转换为 `Future` 状态机：

```rust
// 你写的代码
async fn example() -> u32 {
    let x = fetch().await;  // 点 1
    let y = process(x).await; // 点 2
    x + y
}

// 编译器转换为状态机
enum ExampleFuture {
    Start,
    WaitingForFetch { x: u32 },
    WaitingForProcess { x: u32, y: u32 },
    Done(u32),
}

impl Future for ExampleFuture {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<u32> {
        match self {
            ExampleFuture::Start => {
                // 设置状态为 WaitingForFetch
                // 注册 waker
                Poll::Pending
            }
            ExampleFuture::WaitingForFetch { x } => {
                // 检查 fetch 是否完成
                if ready {
                    *self = ExampleFuture::WaitingForProcess { x: result, y: 0 };
                    Poll::Pending
                } else {
                    Poll::Pending
                }
            }
            // ...
        }
    }
}
```

**关键点**：

- 每个 `.await` 点都是一个状态
- `poll()` 被调用时，如果没准备好，返回 `Poll::Pending`
- 通过 `Waker` 通知调度器任务已就绪

---

### 3. Waker 机制

Waker 是异步编程的核心：

```rust
// Waker 的作用：通知调度器"任务可以继续执行了"

struct Task {
    state: State,
    waker: Option<Waker>,
}

impl Future for Task {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<u32> {
        if self.state.is_ready() {
            Poll::Ready(self.state.result())
        } else {
            // 保存 waker，以便 I/O 完成时通知
            self.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

// 当 I/O 完成时（例如 socket 可读）
fn on_io_ready() {
    // 唤醒保存的 waker
    task.waker.as_ref().unwrap().wake();
    // 调度器会重新 poll 这个任务
}
```

**工作流程**：

```txt
1. 任务 A 调用 socket.read().await
   ↓
2. socket 还没数据，poll 返回 Pending
   ↓
3. 任务 A 保存 waker，注册到 epoll
   ↓
4. 调度器切换到任务 B 执行
   ↓
5. epoll 收到数据通知
   ↓
6. waker.wake() 被调用
   ↓
7. 调度器重新调度任务 A
   ↓
8. 任务 A 再次被 poll，这次能读到数据
```

---

### 4. Reactor 模式

Reactor 负责监控 I/O 事件：

```rust
// 简化的 Reactor 逻辑
struct Reactor {
    epoll_fd: i32,  // Linux epoll 文件描述符
    tasks: HashMap<Token, Task>,
}

impl Reactor {
    fn new() -> Self {
        Self {
            epoll_fd: epoll_create(),
            tasks: HashMap::new(),
        }
    }

    // 注册 I/O 事件
    fn register(&mut self, fd: i32, token: Token, interest: Interest) {
        epoll_ctl(self.epoll_fd, EPOLL_CTL_ADD, fd, &Event::new(EPOLLIN, token));
    }

    // 主事件循环
    fn run(&mut self) {
        loop {
            let events = epoll_wait(self.epoll_fd, &mut events, timeout);
            for event in &events {
                let token = event.token();
                if let Some(task) = self.tasks.get_mut(&token) {
                    task.waker.as_ref().unwrap().wake();
                }
            }
        }
    }
}
```

**平台差异**：

```txt
| 平台 | API | 说明 |
|------|-----|------|
| Linux | epoll | 高效的事件驱动机制 |
| macOS | kqueue | BSD 系统的事件通知 |
| Windows | IOCP | I/O 完成端口 |
```

---

### 5. 调度器工作原理

多线程调度器的工作方式：

```rust
// 简化的调度器逻辑
struct Scheduler {
    // 每个工作线程的本地队列
    local_queues: Vec<Deque<Task>>,
    // 全局队列（所有线程共享）
    global_queue: Arc<Mutex<Vec<Task>>>,
    // 工作线程
    workers: Vec<JoinHandle<()>>,
}

impl Scheduler {
    fn spawn(&mut self, task: Task) {
        // 优先放到本地队列
        let worker_id = current_worker_id();
        self.local_queues[worker_id].push_back(task);
    }

    fn run(&mut self) {
        for worker in &mut self.workers {
            worker.spawn(|| {
                loop {
                    // 1. 尝试从本地队列取任务
                    if let Some(task) = self.local_queues[worker_id].pop_back() {
                        task.poll();
                        continue;
                    }

                    // 2. 尝试从全局队列"偷"任务
                    if let Some(task) = self.steal_from_others() {
                        task.poll();
                        continue;
                    }

                    // 3. 休息，等待 I/O 事件
                    self.park();
                }
            });
        }
    }

    // 工作窃取：从其他线程的队列偷任务
    fn steal_from_others(&self) -> Option<Task> {
        for other_queue in &self.local_queues {
            if let Some(task) = other_queue.pop_front() {
                return Some(task);
            }
        }
        None
    }
}
```

**调度策略**：

1. 优先执行本地队列的任务
2. 窃取其他线程的任务（负载均衡）
3. 没有任务时休眠，等待 I/O 事件唤醒

---

### 6. 任务状态流转

#### 6.1 状态概览

可以先只看图，其余的跳过

```txt
┌─────────────────────────────────────────────────────────────────┐
│                        任务状态机                                │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│    ┌──────────┐    spawn()          ┌──────────────────┐        │
│    │ Created  │ ──────────────────► │   Ready          │        │
│    │  (新建)  │                    │   (就绪)          │        │
│    └──────────┘                    └────────┬─────────┘        │
│                                             │                   │
│                                    调度器选择执行                │
│                                             │                   │
│                                             ▼                   │
│                                    ┌──────────────────┐        │
│                                    │   Running        │        │
│                                    │   (运行中)        │        │
│                                    └────────┬─────────┘        │
│                                             │                   │
│                              ┌──────────────┼──────────────┐    │
│                              │              │              │    │
│                              ▼              ▼              ▼    │
│                        ┌──────────┐  ┌──────────┐  ┌──────────┐│
│                        │ Pending  │  │  Done    │  │ Cancelled││
│                        └──────────┘  └──────────┘  └──────────┘│
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**四种核心状态**：

```txt
| 状态 | 说明 | 何时进入 |
|------|------|----------|
| **Created** | 任务刚被创建 | `spawn()` 调用时 |
| **Ready** | 任务已就绪，等待调度 | 任务创建完成 或 I/O 完成 |
| **Running** | 任务正在执行 | 调度器分配线程 |
| **Pending** | 任务等待资源（I/O、锁等） | `.await` 阻塞时 |
| **Done** | 任务完成 | 返回 `Poll::Ready` |
| **Cancelled** | 任务被取消 | 收到 cancellation 信号 |
```

---

#### 6.2 详细状态转换图

```txt
┌──────────────────────────────────────────────────────────────────────────────┐
│                           详细状态转换                                        │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  spawn(async { ... })                                                        │
│      │                                                                       │
│      ▼                                                                       │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │                              Created                                  │    │
│  │                          (任务已注册到调度器)                           │    │
│  └──────────────────────────────────────────────────────────────────────┘    │
│      │                                                                       │
│      │ 调度器从队列取出任务                                                  │
│      ▼                                                                       │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │                              Ready                                     │    │
│  │                    (等待被 Worker 线程 pick)                           │    │
│  └──────────────────────────────────────────────────────────────────────┘    │
│      │                                                                       │
│      │ Worker 线程调用 poll()                                               │
│      ▼                                                                       │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │                             Running                                    │    │
│  │                          (正在执行代码)                                 │    │
│  │                                                                        │    │
│  │  ┌─────────────────────────────────────────────────────────────────┐  │    │
│  │  │  代码执行流程：                                                   │  │    │
│  │  │                                                                  │  │    │
│  │  │  async {                                                         │  │    │
│  │  │      let data = socket.read().await;  ◄─── 进入 Pending         │  │    │
│  │  │      process(data);                      ◄─── 返回 Running       │  │    │
│  │  │      let result = db.write(data).await; ◄─── 进入 Pending       │  │    │
│  │  │      println!("{}", result);             ◄─── 返回 Running       │  │    │
│  │  │  }  // 函数结束 → 进入 Done                                      │  │    │
│  │  │                                                                  │  │    │
│  │  └─────────────────────────────────────────────────────────────────┘  │    │
│  │                                                                        │    │
│  └──────────────────────────────────────────────────────────────────────┘    │
│      │                                                                       │
│      ├──────────────────────────────────────────────────────────────┐       │
│      │                              │                              │       │
│      │                              ▼                              │       │
│      │              ┌─────────────────────────────────┐            │       │
│      │              │         Pending                 │            │       │
│      │              │        (等待 I/O/锁/定时器)      │            │       │
│      │              └───────────────┬─────────────────┘            │       │
│      │                              │                              │       │
│      │              ┌───────────────┼───────────────┐              │       │
│      │              │               │               │              │       │
│      │              ▼               ▼               ▼              │       │
│      │        ┌──────────┐  ┌──────────┐  ┌──────────┐            │       │
│      │        │  I/O     │  │  Timer   │  │  Mutex   │            │       │
│      │        │ Pending  │  │ Pending  │  │ Pending  │            │       │
│      │        └────┬─────┘  └────┬─────┘  └────┬─────┘            │       │
│      │             │             │             │                    │       │
│      │             │             │             │                    │       │
│      │             │             │             │                    │       │
│      │             │  wake()     │  wake()     │  unlock()         │       │
│      │             │             │             │                   │       │
│      │             │             │             │                   │       │
│      └─────────────┴─────────────┴─────────────┴───────────────────┘       │
│      │                                                                       │
│      │ 所有 .await 完成，poll() 返回 Ready                                  │
│      ▼                                                                       │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │                              Done                                      │    │
│  │                         (任务执行完毕)                                  │    │
│  │                                                                        │    │
│  │  handle.await 获取返回值                                               │    │
│  │                                                                        │    │
│  └──────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐    │
│  │                          Cancelled                                     │    │
│  │                       (任务被提前取消)                                  │    │
│  │                                                                        │    │
│  │  drop(handle);  // 取消任务                                            │    │
│  │                                                                        │    │
│  └──────────────────────────────────────────────────────────────────────┘    │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

#### 6.3 实际代码示例

```rust
#[tokio::main]
async fn main() {
    // 1. spawn 创建任务 → Created
    let handle = tokio::spawn(async {
        println!("1. 任务开始执行");

        // 2. 进入 Pending（等待 I/O）
        println!("2. 等待数据...");
        let data = socket_read().await;  // 状态: Ready → Pending

        // 4. 恢复执行（I/O 完成）
        println!("3. 收到数据: {:?}", data);
        process(data);

        // 5. 再次进入 Pending（等待定时器）
        println!("4. 等待定时器...");
        tokio::time::sleep(Duration::from_secs(1)).await;  // 状态: Running → Pending

        // 7. 定时器完成，恢复执行
        println!("5. 定时器完成");

        // 8. 任务完成
        "result".to_string()
    });

    // 任务状态流转：
    // Created → Ready → Running → Pending → Running → Pending → Running → Done
    let result = handle.await.unwrap();
    println!("6. 任务结果: {}", result);
}

// 模拟 I/O 操作
async fn socket_read() -> Vec<u8> {
    // 这会注册到 Reactor，返回 Pending
    // 当数据到达时，Waker 被调用
    vec![1, 2, 3]
}
```

---

#### 6.4 poll() 方法详解

`poll()` 是任务状态转换的核心：

```rust
// Future trait 的核心方法
trait Future {
    type Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;
}
```

**poll() 的职责**：

```txt
┌─────────────────────────────────────────────────────────────────┐
│                       poll() 执行流程                            │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  输入:                                                           │
│  ├── self: Pin<&mut Self>  (固定内存位置，防止自引用)            │
│  └── cx: &mut Context    (包含 Waker)                           │
│                                                                 │
│  输出:                                                           │
│  ├── Poll::Ready(value)  → 任务完成，返回结果                    │
│  └── Poll::Pending       → 任务还没准备好                       │
│                                                                 │
│  内部逻辑:                                                        │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                                                         │   │
│  │  1. 检查任务状态                                         │   │
│  │     ├── 如果已完成 → 返回 Ready                          │   │
│  │     └── 如果未完成 → 继续                                │   │
│  │                                                         │   │
│  │  2. 执行任务代码                                         │   │
│  │     ├── 遇到 .await → 保存 Waker，返回 Pending          │   │
│  │     └── 执行完毕 → 返回 Ready                            │   │
│  │                                                         │   │
│  │  3. 处理 Waker                                           │   │
│  │     └── 如果需要，克隆 Waker 用于后续唤醒                │   │
│  │                                                         │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Waker 的传递**：

```rust
// 简化版的 I/O Future
struct SocketRead {
    socket: TcpSocket,
    data: Option<Vec<u8>>,
    waker: Option<Waker>,
}

impl Future for SocketRead {
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Vec<u8>> {
        match &mut self.data {
            Some(data) => {
                // 数据已准备好
                Poll::Ready(data.clone())
            }
            None => {
                // 数据还没好，注册 Waker
                self.waker = Some(cx.waker().clone());

                // 注册到操作系统的 epoll
                self.socket.register_for_read();

                Poll::Pending
            }
        }
    }
}

// 当数据到达时，epoll 通知
fn on_socket_data_ready(socket: &TcpSocket) {
    // 唤醒之前保存的 Waker
    if let Some(waker) = &socket_read_future.waker {
        waker.wake();  // 调度器会重新 poll 这个 Future
    }
}
```

---

#### 6.5 Pending 的多种类型

任务进入 Pending 状态的原因有很多：

```rust
async fn example() {
    // 1. I/O 等待
    let mut file = File::open("data.txt").await.unwrap();
    // 状态: Running → I/O Pending

    // 2. 定时器等待
    tokio::time::sleep(Duration::from_secs(5)).await;
    // 状态: Running → Timer Pending

    // 3. 锁等待
    let _guard = mutex.lock().await;
    // 状态: Running → Mutex Pending

    // 4. Channel 等待
    let msg = rx.recv().await;
    // 状态: Running → Channel Pending

    // 5. 其他任务完成
    let result = join!(task1, task2).await;
    // 状态: Running → Join Pending
}
```

**Pending 状态的特点**：

```txt
┌─────────────────────────────────────────────────────────────────┐
│                       Pending 状态详情                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  I/O Pending                                                    │
│  ├── 触发：socket.read(), file.read()                           │
│  ├── 唤醒：数据到达 / 连接建立                                   │
│  └── 唤醒者：Reactor (epoll/kqueue/IOCP)                        │
│                                                                 │
│  Timer Pending                                                  │
│  ├── 触发：sleep(), timeout(), interval                         │
│  ├── 唤醒：定时器到期                                           │
│  └── 唤醒者：Tokio time 模块                                    │
│                                                                 │
│  Mutex Pending                                                  │
│  ├── 触发：mutex.lock()                                         │
│  ├── 唤醒：锁被释放                                             │
│  └── 唤醒者：持有锁的任务                                       │
│                                                                 │
│  Channel Pending                                                │
│  ├── 触发：rx.recv(), tx.send() (buffer 满时)                   │
│  ├── 唤醒：有数据 / 有空闲 buffer                               │
│  └── 唤醒者：发送者或接收者                                     │
│                                                                 │
│  Join Pending                                                   │
│  ├── 触发：join!(), select!                                     │
│  ├── 唤醒：所有/任一任务完成                                    │
│  └── 唤醒者：Tokio 调度器                                       │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

#### 6.6 任务取消 (Cancellation)

任务可能在完成前被取消：

```rust
#[tokio::main]
async fn main() {
    // 创建长时间运行的任务
    let handle = tokio::spawn(async {
        for i in 0..100 {
            println!("Processing {}", i);

            // 每次循环检查是否被取消
            // tokio::select! 可以配合 abort_on_cancel 使用
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        "done"
    });

    // 1 秒后取消任务
    tokio::time::sleep(Duration::from_secs(1)).await;
    drop(handle);  // 取消任务

    // 任务状态: Running → Cancelled
}
```

**取消的两种方式**：

```txt
| 方式 | 代码 | 效果 |
|------|------|------|
| `drop(handle)` | `drop(handle)` | 任务立即被标记为取消 |
| `handle.abort()` | `handle.abort()` | 同上，效果相同 |
```

**取消后的行为**：

```rust
let handle = tokio::spawn(async {
    // 这个任务会被取消
    some_long_running_task().await
});

handle.abort();  // 取消

// 等待任务完成（会返回 Aborted 错误）
let result = handle.await;

// result 是 Err(JoinError::Aborted)
```

---

#### 6.7 完整的任务生命周期

```txt
┌──────────────────────────────────────────────────────────────────────────────┐
│                           任务完整生命周期                                     │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  时间 ──────────────────────────────────────────────────────────────────►    │
│                                                                              │
│  spawn()                                                                      │
│    │                                                                         │
│    ▼                                                                         │
│  ┌─────────┐                                                                  │
│  │ Created │  任务对象创建，Future 已注册                                      │
│  └────┬────┘                                                                  │
│       │                                                                        │
│       ▼                                                                        │
│  ┌─────────┐     Worker pick 任务      ┌─────────┐                            │
│  │  Ready  │ ───────────────────────► │ Running │                            │
│  └─────────┘                          └────┬────┘                            │
│                                            │                                  │
│                          ┌─────────────────┼─────────────────┐               │
│                          │                 │                 │               │
│                          ▼                 ▼                 ▼               │
│                    ┌────────────┐     ┌──────────────┐     ┌──────────────┐  │
│                    │ I/O PENDING│     │ Timer PENDING│     │ Mutex PENDING│  │
│                    └─────┬──────┘     └────┬─────────┘     └─┬────────────┘  │
│                          │                 │                 │               │
│                          │                 │                 │               │
│                          │  wake()         │  wake()         │  unlock()     │
│                          │                 │                 │               │
│                          └─────────────────┴─────────────────┘               │
│                                            │                                  │
│                                            ▼                                  │
│                                    ┌─────────────┐                           │
│                                    │   Running   │                           │
│                                    └──────┬──────┘                           │
│                                           │                                  │
│                    ┌──────────────────────┼──────────────────────┐           │
│                    │                      │                      │           │
│                    ▼                      ▼                      ▼           │
│              ┌──────────┐          ┌──────────┐          ┌──────────┐       │
│              │  Done    │          │ Cancelled│          │  Panic   │       │
│              └──────────┘          └──────────┘          └──────────┘       │
│                                                                              │
│  handle.await ◄────────────────────────────────────────────────────────────  │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

#### 6.8 常见问题

**Q: 一个任务可以被 poll 多少次？**

**A**: 理论上无限次，直到完成。但实际中：

- 每次 Pending 后最多 poll 一次（被 Waker 唤醒后）
- 通常一个任务的生命周期中 poll 次数 = `.await` 点数 + 1（初始化）

**Q: Waker 为什么必须克隆？**

**A**: 因为 Waker 是 `&Waker` 引用，而任务需要保存它用于以后唤醒：

```rust
fn poll(&mut self, cx: &mut Context<'_>) -> Poll<...> {
    // cx.waker() 返回 &Waker，但 self 需要长期保存
    // 所以必须 clone()
    self.waker = Some(cx.waker().clone());
    Poll::Pending
}
```

**Q: 任务在 Pending 状态时会占用 CPU 吗？**

**A**: 不会！Pending 意味着任务被移出执行队列，不会被调度执行，直到 Waker 被调用。

---

#### 6.9 一句话记住状态流转

```txt
Created → Ready → Running → Pending → Running → Done/Cancelled
                ↑              │
                └──────────────┘
                (Waker 唤醒)
```

**关键点**：任务在 Running 和 Pending 之间反复切换，直到完成或被取消。

---

### 7. spawn vs block_on

```rust
#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // spawn：在调度器中注册任务，立即返回
    // 任务会被放入队列，由工作线程执行
    let handle = tokio::spawn(async {
        heavy_computation().await
    });

    // 可以继续做其他事...
    println!("Spawned task!");

    // 等待任务完成
    let result = handle.await.unwrap();

    // block_on：阻塞当前线程，等待 Future 完成
    // 会切换到调度器执行其他任务，但当前线程被阻塞
    let sync_result = tokio::task::block_on({
        async {
            // 这个 async 块会在调度器中运行
            // 但 block_on 会阻塞调用线程
            some_async_work().await
        }
    });
}
```

**对比**：

```txt
| 特性 | `spawn` | `block_on` |
|------|---------|------------|
| 阻塞线程 | 否 | 是 |
| 返回时机 | 立即返回 | 等待完成 |
| 并发性 | 高（多个任务） | 低（单线程等待） |
| 使用场景 | 并发处理 | 桥接同步代码 |
```

---

### 8. 为什么 Tokio 高效？

```txt
传统多线程模型：              Tokio 异步模型：
─────────────────           ─────────────────
线程 1: [████████░░]        线程 1: [█░░█░░█░░]
       等待 I/O                   切换任务

线程 2: [░░░░░░████]        线程 1: [░░█░░█░░█]
       等待 I/O                   继续执行

线程 3: [████░░░░░░]        线程 1: [█░░█░░█░░]
       等待 I/O                   继续执行

总计：3 线程都阻塞           总计：1 线程处理所有任务
```

**优势**：

1. **更少的线程**：几个线程就能处理成千上万个并发任务
2. **更少的上下文切换**：线程少，上下文切换开销小
3. **更好的缓存局部性**：任务在同一个线程上运行
4. **无锁/少锁**：基于消息传递，避免共享状态

---

### 9. 内存模型

```txt
每个 spawn 的任务大约占用：
- 状态机：几百字节
- 栈空间：2KB（可配置）
- Waker：几十字节

vs 操作系统线程：
- 栈空间：1MB（固定）
- 上下文：几 KB

所以 Tokio 能轻松支持百万级并发任务！
```

---

### 10. 一句话理解原理

```txt
Future = 状态机（保存暂停位置）
Waker = 回调（通知"我准备好了"）
Reactor = 监控器（监听 I/O 事件）
Scheduler = 调度器（分配任务到线程）
```

四个组件配合，实现"单线程处理大量 I/O"的魔法。

---

## Monad 中的 Tokio 用法

### 1. 多线程运行时设置

```rust
// monad-node/src/main.rs
let runtime = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .context("Failed to build tokio runtime")?;
```

**配置说明**：

```rust
Builder::new_multi_thread()
    .worker_threads(8)      // 工作线程数
    .enable_all()           // 启用 I/O、时间、信号驱动
    .max_blocking_threads(256) // 阻塞线程数上限
    .build()
```

---

### 2. 在 Monad 中的作用

```txt
┌─────────────────────────────────────────────────┐
│              Tokio 在 Monad 中的用途             │
├─────────────────────────────────────────────────┤
│  网络通信                                        │
│  ├── RaptorCast (p2p 网络)                      │
│  ├── gRPC (RPC 调用)                            │
│  └── WebSocket (API 接口)                       │
├─────────────────────────────────────────────────┤
│  状态同步                                        │
│  ├── 区块同步                                    │
│  └── 交易传播                                    │
├─────────────────────────────────────────────────┤
│  交易池处理                                      │
│  ├── 交易广播                                    │
│  └── 交易验证                                    │
├─────────────────────────────────────────────────┤
│  控制面板 IPC                                    │
├─────────────────────────────────────────────────┤
│  指标收集                                        │
│  └── OpenTelemetry                              │
└─────────────────────────────────────────────────┘
```

---

### 3. 异步频道 (Channel)

```rust
use tokio::sync::{mpsc, oneshot};

// 单发送者 + 单接收者
let (tx, mut rx) = mpsc::channel::<Message>(100);

// 发送消息
tx.send(Message::Hello).await?;

// 接收消息
while let Some(msg) = rx.recv().await {
    println!("Received: {:?}", msg);
}
```

**类型对比**：

```txt
| Channel 类型 | 特点 |
|--------------|------|
| `mpsc` | 多生产者单消费者 |
| `watch` | 多消费者广播最新值 |
| `oneshot` | 单次发送+接收 |
```

---

### 4. 互斥锁 (Mutex)

```rust
use tokio::sync::Mutex;
use std::sync::Arc;

let counter = Arc::new(Mutex::new(0));

let handles: Vec<_> = (0..10).map(|_| {
    let counter = Arc::clone(&counter);
    tokio::spawn(async move {
        let mut num = counter.lock().await;
        *num += 1;
    })
}).collect();

futures::future::join_all(handles).await;

println!("Result: {}", *counter.lock().await);
```

**注意**：

- `tokio::sync::Mutex` 是异步安全的
- 不要用 `std::sync::Mutex` 在 async 代码中！

---

## async/await 语法

### 基本用法

```rust
async fn hello() -> String {
    "Hello, Tokio!".to_string()
}

#[tokio::main]
async fn main() {
    // 调用异步函数
    let result = hello().await;
    println!("{}", result);
}
```

### 语法糖转换

```rust
// 你写的
async fn my_func() -> u32 {
    42
}

// 编译器转换为
fn my_func() -> impl Future<Output = u32> {
    async { 42 }
}
```

---

## Tokio vs Rayon

```txt
┌──────────────────────────────────────────────────────────────┐
│                      Tokio vs Rayon                           │
├────────────────────┬─────────────────────────────────────────┤
│      Tokio         │              Rayon                      │
├────────────────────┼─────────────────────────────────────────┤
│ 异步 I/O           │ 并行计算                                 │
│ 事件循环驱动       │ 工作窃取调度                             │
│ 协程 (async/await) │ 并行迭代器                               │
│ 处理网络/文件 I/O  │ 处理大量数据                             │
│ I/O 密集型         │ CPU 密集型                               │
│ 任务挂起/恢复      │ 同时执行多个任务                         │
├────────────────────┴─────────────────────────────────────────┤
│                    配合使用                                    │
├──────────────────────────────────────────────────────────────┤
│  Tokio: 网络通信、状态同步、IPC                              │
│  Rayon: 交易验证、签名恢复、区块验证                          │
│  两者配合 = 完整的异步 + 并行运行时                           │
└──────────────────────────────────────────────────────────────┘
```

---

## 常用 API

### 定时器

```rust
use tokio::time;

// 睡眠
time::sleep(Duration::from_secs(1)).await;

// 定时任务
time::interval(Duration::from_secs(1)).tick().for_each(|_| {
    println!("每秒执行");
});

// 超时
let result = time::timeout(Duration::from_secs(5), long_operation()).await?;
```

### I/O 操作

```rust
use tokio::fs;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};

// 异步读取文件
let mut file = fs::File::open("foo.txt").await?;
let mut contents = String::new();
file.read_to_string(&mut contents).await?;

// 异步写入
let mut file = fs::File::create("bar.txt").await?;
file.write_all(b"Hello world").await?;
```

### TCP 网络

```rust
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// 服务端
let listener = TcpListener::bind("127.0.0.1:8080").await?;
loop {
    let (socket, _) = listener.accept().await?;
    tokio::spawn(async move {
        // 处理连接
    });
}
```

---

## Java 对照表

```txt
| Rust (Tokio) | Java | 说明 |
|--------------|------|------|
| `Runtime` | `Executor` | 任务调度器 |
| `async/await` | `CompletableFuture` | 异步任务 |
| `.await` | `.get()` / `.join()` | 等待结果 |
| `tokio::spawn` | `executor.submit()` | 提交任务 |
| `tokio::join!` | `CompletableFuture.allOf()` | 并发等待 |
| `mpsc channel` | `BlockingQueue` | 线程通信 |
| `Mutex` | `ReentrantLock` | 互斥锁 |
| `oneshot` | `Promise` | 单次通信 |
| `select!` | `CompletableFuture.anyOf()` | 多任务选择 |
```

---

## 常见问题

### Q: 什么时候用 Tokio？什么时候用 Rayon？

```rust
// 用 Tokio：I/O 密集型
async fn fetch_urls(urls: &[&str]) {
    for url in urls {
        let response = reqwest::get(url).await?;  // 网络 I/O
    }
}

// 用 Rayon：CPU 密集型
fn process_blocks(blocks: &[Block]) {
    blocks.par_iter().for_each(|block| {
        verify_signature(block);  // CPU 计算
    });
}
```

### Q: `block_on` vs `spawn`？

```rust
#[tokio::main]
async fn main() {
    // block_on：阻塞当前线程等待结果
    let result = tokio::task::block_on(async {
        // 这里会阻塞整个线程
        some_async_operation().await
    });

    // spawn：在后台运行任务
    let handle = tokio::spawn(async {
        some_async_operation().await
    });
    // 继续做其他事...
}
```

### Q: 如何调试异步代码？

```rust
use tracing;

// 添加追踪
#[tracing::instrument]
async fn my_function() {
    // 自动记录调用参数和耗时
}
```

### Q: panic 会怎样？

```rust
// 默认：panic 会关闭整个 runtime！
tokio::spawn(async {
    panic!("oops!");
});

// 捕获 panic
let result = tokio::spawn(async { panic!("oops") })
    .await;
```

---

## 学习路径

1. **基础**：`async`/`await` 语法
2. **Runtime**：`#[tokio::main]` 理解运行时
3. **任务**：`spawn`、`join!`、`select!`
4. **同步**：`Mutex`、`Channel`、`Arc`
5. **I/O**：`TcpListener`、`fs`、`time`
6. **进阶**：`select!` 模式、`cancellation`

---

## 一句话总结

```txt
Tokio = 写同步代码的方式，写异步代码
```

把 `.await` 当作"等一下，但不阻塞线程"来理解就行。

---

## 学习资源

1. 官方文档：https://tokio.rs/tokio/tutorial
2. 中文教程：https://course.rs/async-rust/tokio.html
3. Async Book：https://rust-lang.github.io/async-book/
