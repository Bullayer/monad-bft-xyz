use monad_rocks_db::{RocksDBStore, RocksDBConfig, ListIndexMut, ListIndex, Fork, Storage};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    println!("=== RocksDB 配置和列族 ===");
    let _config = RocksDBConfig::high_performance();
    let mut cf_storage = RocksDBStore::with_column_families("./db_cf", &["users", "posts"])?;
    
    cf_storage.put_cf("users", "user1", b"Alice".to_vec())?;
    cf_storage.put_cf("users", "user2", b"Bob".to_vec())?;
    
    cf_storage.put_cf("posts", "post1", b"Hello World".to_vec())?;
    
    println!("User1: {:?}", cf_storage.get_cf("users", "user1"));
    println!("Post1: {:?}", cf_storage.get_cf("posts", "post1"));
    println!("列族列表: {:?}", cf_storage.list_column_families());
    
    println!("\n=== 使用默认列族 ===");
    let mut storage = RocksDBStore::new("./db")?;
    
    println!("=== 基本操作 ===");
    storage.put("mykey", b"myvalue".to_vec());
    let snapshot = storage.snapshot();
    match snapshot.get("mykey") {
        Some(v) => println!("检索到的值: {}", String::from_utf8(v)?),
        None => println!("未找到值"),
    }

    println!("\n=== 使用 ListIndex ===");
    {
        let mut list = ListIndexMut::new(&mut storage, "numbers");
        list.push(42u32);
        list.push(100u32);
        list.push(200u32);
        list.commit();
    }
    
    let snapshot = storage.snapshot();
    let list: ListIndex<u32> = ListIndex::new(&*snapshot, "numbers");
    println!("列表长度: {}", list.len());
    for i in 0..list.len() {
        if let Some(value) = list.get(i) {
            println!("  [{}] = {}", i, value);
        }
    }

    println!("\n=== 使用 Fork ===");
    storage.put("original", b"original_value".to_vec());
    
    let mut fork = Fork::new(&storage);
    fork.put("original", b"forked_value".to_vec());
    fork.put("new_key", b"new_value".to_vec());
    
    // 在 fork 中查看
    println!("Fork 中的 original: {:?}", fork.get("original"));
    println!("Fork 中的 new_key: {:?}", fork.get("new_key"));
    
    // 原始存储未改变
    let snapshot = storage.snapshot();
    println!("原始存储中的 original: {:?}", snapshot.get("original"));
    
    // 应用 fork 的更改
    let patch = fork.into_patch();
    patch.apply(&mut storage);
    
    let snapshot = storage.snapshot();
    println!("应用后存储中的 original: {:?}", snapshot.get("original"));
    println!("应用后存储中的 new_key: {:?}", snapshot.get("new_key"));

    Ok(())
}
