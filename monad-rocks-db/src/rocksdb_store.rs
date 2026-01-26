use crate::{Snapshot, Storage};
use rust_rocksdb::{DB, Options, ColumnFamilyDescriptor, ReadOptions};
use std::path::Path;
use std::sync::Arc;
use std::fmt;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct RocksDBStoreError {
    message: String,
}

impl RocksDBStoreError {
    fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl fmt::Display for RocksDBStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RocksDBStoreError {}

impl From<rust_rocksdb::Error> for RocksDBStoreError {
    fn from(err: rust_rocksdb::Error) -> Self {
        Self::new(format!("RocksDB error: {}", err))
    }
}

// RocksDB 数据库级别配置选项
// 用于自定义 RocksDB 数据库级别的行为和性能参数
#[derive(Clone, Debug)]
pub struct RocksDBConfig {
    // 是否在数据库不存在时创建
    pub create_if_missing: bool,
    // 是否在列族不存在时创建
    pub create_missing_column_families: bool,
    // 是否启用错误日志
    pub error_if_exists: bool,
    // 最大打开文件数
    pub max_open_files: i32,
    // 写缓冲区大小（字节）
    pub write_buffer_size: usize,
    // 最大写缓冲区数量
    pub max_write_buffer_number: i32,
    // 目标文件大小（字节）
    pub target_file_size_base: u64,
    // 最小写缓冲区合并数量
    pub min_write_buffer_number_to_merge: i32,
    // 压缩类型
    pub compression_type: Option<rust_rocksdb::DBCompressionType>,
    // 布隆过滤器位数
    pub bloom_filter_bits_per_key: Option<u32>,
    // 最大后台压缩任务数（-1 表示自动）
    pub max_background_compactions: Option<i32>,
    // 最大后台刷新任务数（-1 表示自动）
    pub max_background_flushes: Option<i32>,
    // 最大后台任务数
    pub max_background_jobs: Option<i32>,
    // 每次同步的字节数
    pub bytes_per_sync: Option<u64>,
    // WAL 每次同步的字节数
    pub wal_bytes_per_sync: Option<u64>,
    // 延迟写入速率（字节/秒）- 注意：此选项在 rust-rocksdb 中可能不可用
    pub delayed_write_rate: Option<u64>,
    // 是否启用严格字节同步 - 注意：此选项在 rust-rocksdb 中可能不可用
    pub strict_bytes_per_sync: Option<bool>,
    // 是否启用偏执检查
    pub paranoid_checks: Option<bool>,
    // 是否启用并发 memtable 写入
    pub allow_concurrent_memtable_write: Option<bool>,
    // 是否启用原子刷新
    pub atomic_flush: Option<bool>,
    // 是否启用手动 WAL 刷新
    pub manual_wal_flush: Option<bool>,
    // 最大子压缩数
    pub max_subcompactions: Option<u32>,
    // 表缓存分片位数
    pub table_cache_numshardbits: Option<i32>,
    // 最大文件打开线程数
    pub max_file_opening_threads: Option<i32>,
    // WAL 恢复模式
    pub wal_recovery_mode: Option<rust_rocksdb::DBRecoveryMode>,
}

// 列族级别配置选项
// 用于为每个列族单独配置参数
#[derive(Clone, Debug)]
pub struct ColumnFamilyConfig {
    // 写缓冲区大小（字节）
    pub write_buffer_size: Option<usize>,
    // 最大写缓冲区数量
    pub max_write_buffer_number: Option<i32>,
    // 目标文件大小（字节）
    pub target_file_size_base: Option<u64>,
    // 最小写缓冲区合并数量
    pub min_write_buffer_number_to_merge: Option<i32>,
    // 压缩类型
    pub compression_type: Option<rust_rocksdb::DBCompressionType>,
    // 布隆过滤器位数
    pub bloom_filter_bits_per_key: Option<u32>,
    // 最大子压缩数
    pub max_subcompactions: Option<u32>,
    // 是否启用前缀提取器
    pub prefix_extractor: Option<bool>,
}

impl Default for RocksDBConfig {
    fn default() -> Self {
        Self {
            create_if_missing: true,
            create_missing_column_families: true,
            error_if_exists: false,
            max_open_files: 512,
            write_buffer_size: 64 * 1024 * 1024,
            max_write_buffer_number: 3,
            target_file_size_base: 64 * 1024 * 1024,
            min_write_buffer_number_to_merge: 1,
            compression_type: Some(rust_rocksdb::DBCompressionType::Snappy),
            bloom_filter_bits_per_key: Some(10u32),
            max_background_compactions: None,
            max_background_flushes: None,
            max_background_jobs: None,
            bytes_per_sync: None,
            wal_bytes_per_sync: None,
            delayed_write_rate: None,
            strict_bytes_per_sync: None,
            paranoid_checks: None,
            allow_concurrent_memtable_write: None,
            atomic_flush: None,
            manual_wal_flush: None,
            max_subcompactions: None,
            table_cache_numshardbits: None,
            max_file_opening_threads: None,
            wal_recovery_mode: None,
        }
    }
}

impl Default for ColumnFamilyConfig {
    fn default() -> Self {
        Self {
            write_buffer_size: None,
            max_write_buffer_number: None,
            target_file_size_base: None,
            min_write_buffer_number_to_merge: None,
            compression_type: None,
            bloom_filter_bits_per_key: None,
            max_subcompactions: None,
            prefix_extractor: None,
        }
    }
}

impl RocksDBConfig {
    pub fn high_performance() -> Self {
        Self {
            write_buffer_size: 128 * 1024 * 1024,
            max_write_buffer_number: 6,
            min_write_buffer_number_to_merge: 2,
            ..Default::default()
        }
    }

    pub fn low_memory() -> Self {
        Self {
            max_open_files: 256,
            write_buffer_size: 16 * 1024 * 1024,
            max_write_buffer_number: 2,
            ..Default::default()
        }
    }

    fn apply_to_db_options(&self, opts: &mut Options) {
        opts.create_if_missing(self.create_if_missing);
        opts.create_missing_column_families(self.create_missing_column_families);
        opts.set_error_if_exists(self.error_if_exists);
        opts.set_max_open_files(self.max_open_files);
        
        if let Some(val) = self.max_background_jobs {
            opts.set_max_background_jobs(val);
        }
        if let Some(val) = self.bytes_per_sync {
            opts.set_bytes_per_sync(val);
        }
        if let Some(val) = self.wal_bytes_per_sync {
            opts.set_wal_bytes_per_sync(val);
        }
        if let Some(val) = self.paranoid_checks {
            opts.set_paranoid_checks(val);
        }
        if let Some(val) = self.allow_concurrent_memtable_write {
            opts.set_allow_concurrent_memtable_write(val);
        }
        if let Some(val) = self.atomic_flush {
            opts.set_atomic_flush(val);
        }
        if let Some(val) = self.manual_wal_flush {
            opts.set_manual_wal_flush(val);
        }
        if let Some(val) = self.max_subcompactions {
            opts.set_max_subcompactions(val);
        }
        if let Some(val) = self.table_cache_numshardbits {
            opts.set_table_cache_num_shard_bits(val);
        }
        if let Some(val) = self.max_file_opening_threads {
            opts.set_max_file_opening_threads(val);
        }
        if let Some(val) = self.wal_recovery_mode {
            opts.set_wal_recovery_mode(val);
        }
    }

    fn apply_to_cf_options(&self, opts: &mut Options) {
        opts.set_write_buffer_size(self.write_buffer_size);
        opts.set_max_write_buffer_number(self.max_write_buffer_number);
        opts.set_target_file_size_base(self.target_file_size_base);
        opts.set_min_write_buffer_number_to_merge(self.min_write_buffer_number_to_merge);
        
        if let Some(compression) = self.compression_type {
            opts.set_compression_type(compression);
        }
        
        if let Some(bits) = self.bloom_filter_bits_per_key {
            opts.set_bloom_locality(bits);
        }
    }
}

impl ColumnFamilyConfig {
    fn apply_to_options(&self, base_opts: &mut Options) {
        if let Some(val) = self.write_buffer_size {
            base_opts.set_write_buffer_size(val);
        }
        if let Some(val) = self.max_write_buffer_number {
            base_opts.set_max_write_buffer_number(val);
        }
        if let Some(val) = self.target_file_size_base {
            base_opts.set_target_file_size_base(val);
        }
        if let Some(val) = self.min_write_buffer_number_to_merge {
            base_opts.set_min_write_buffer_number_to_merge(val);
        }
        if let Some(compression) = self.compression_type {
            base_opts.set_compression_type(compression);
        }
        if let Some(bits) = self.bloom_filter_bits_per_key {
            base_opts.set_bloom_locality(bits);
        }
        if let Some(val) = self.max_subcompactions {
            base_opts.set_max_subcompactions(val);
        }
    }
}

pub struct RocksDBStore {
    db: Arc<DB>,
    column_family_names: Vec<String>,
}

impl RocksDBStore {
    // 创建新的 RocksDB 存储实例（使用默认列族）
    // # 参数
    // * `path` - 数据库文件存储路径
    // 
    // # 示例
    // ```no_run
    // # fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let storage = RocksDBStore::new("./my_db")?;
    // # Ok(())
    // # }
    // ```
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, RocksDBStoreError> {
        Self::with_config(path, RocksDBConfig::default(), &[])
    }

    // 使用自定义配置创建 RocksDB 存储实例
    // 
    // # 参数
    // * `path` - 数据库文件存储路径
    // * `config` - RocksDB 配置选项
    // 
    // # 示例
    // ```no_run
    // # fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let config = RocksDBConfig::high_performance();
    // let storage = RocksDBStore::with_config("./my_db", config, &[])?;
    // # Ok(())
    // # }
    // ```
    pub fn with_config<P: AsRef<Path>>(
        path: P,
        config: RocksDBConfig,
        column_families: &[&str],
    ) -> Result<Self, RocksDBStoreError> {
        Self::with_config_and_cf_configs(path, config, column_families, HashMap::new())
    }

    // 使用自定义配置和列族配置创建 RocksDB 存储实例
    // 
    // # 参数
    // * `path` - 数据库文件存储路径
    // * `config` - RocksDB 数据库级别配置选项
    // * `column_families` - 列族名称列表
    // * `cf_configs` - 列族名称到配置的映射，如果某个列族没有配置，将使用数据库配置的默认值
    // 
    // # 示例
    // ```no_run
    // use std::collections::HashMap;
    // # fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let mut config = RocksDBConfig::default();
    // config.max_background_compactions = Some(4);
    // 
    // let mut cf_configs = HashMap::new();
    // let mut users_cf = ColumnFamilyConfig::default();
    // users_cf.write_buffer_size = Some(128 * 1024 * 1024);
    // users_cf.compression_type = Some(rust_rocksdb::DBCompressionType::Lz4);
    // cf_configs.insert("users".to_string(), users_cf);
    // 
    // let storage = RocksDBStore::with_config_and_cf_configs(
    //     "./my_db", 
    //     config, 
    //     &["users", "posts"], 
    //     cf_configs
    // )?;
    // # Ok(())
    // # }
    // ```
    pub fn with_config_and_cf_configs<P: AsRef<Path>>(
        path: P,
        config: RocksDBConfig,
        column_families: &[&str],
        cf_configs: HashMap<String, ColumnFamilyConfig>,
    ) -> Result<Self, RocksDBStoreError> {
        let mut db_opts = Options::default();
        config.apply_to_db_options(&mut db_opts);

        // 尝试获取现有的列族列表（如果数据库已存在）
        let _existing_cfs = DB::list_cf(&db_opts, &path).unwrap_or_else(|_| Vec::new());
        
        // 准备列族描述符
        let mut cf_descriptors = Vec::new();
        
        // 添加默认列族
        let mut default_cf_opts = Options::default();
        config.apply_to_cf_options(&mut default_cf_opts);
        // 如果为 default 列族指定了配置，应用它
        if let Some(cf_config) = cf_configs.get("default") {
            cf_config.apply_to_options(&mut default_cf_opts);
        }
        cf_descriptors.push(ColumnFamilyDescriptor::new("default", default_cf_opts));
        
        // 添加用户指定的列族
        for cf_name in column_families {
            let mut cf_opts = Options::default();
            // 先应用数据库级别的列族配置（作为基础）
            config.apply_to_cf_options(&mut cf_opts);
            // 如果为该列族指定了单独配置，应用它（会覆盖基础配置）
            if let Some(cf_config) = cf_configs.get(*cf_name) {
                cf_config.apply_to_options(&mut cf_opts);
            }
            cf_descriptors.push(ColumnFamilyDescriptor::new(*cf_name, cf_opts));
        }

        // 打开数据库
        let db = DB::open_cf_descriptors(&db_opts, path, cf_descriptors)?;
        let db = Arc::new(db);

        // 收集所有列族名称（从实际打开的数据库中获取）
        let column_family_names = db.cf_names();

        Ok(Self {
            db,
            column_family_names,
        })
    }

    // 使用列族创建 RocksDB 存储实例
    // 
    // # 参数
    // * `path` - 数据库文件存储路径
    // * `column_families` - 列族名称列表
    // 
    // # 示例
    // ```no_run
    // # fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let storage = RocksDBStore::with_column_families("./my_db", &["users", "posts"])?;
    // # Ok(())
    // # }
    // ```
    pub fn with_column_families<P: AsRef<Path>>(
        path: P,
        column_families: &[&str],
    ) -> Result<Self, RocksDBStoreError> {
        Self::with_config(path, RocksDBConfig::default(), column_families)
    }

    // 从现有的 DB 实例创建
    // 如果你已经有一个 RocksDB 实例，可以使用这个方法来创建存储。
    pub fn from_db(db: Arc<DB>) -> Result<Self, RocksDBStoreError> {
        // 验证默认列族存在
        db.cf_handle("default")
            .ok_or_else(|| RocksDBStoreError::new("Default column family not found"))?;

        // 获取所有列族名称
        let column_family_names = db.cf_names();

        Ok(Self {
            db,
            column_family_names,
        })
    }

    // 获取指定列族的句柄
    // # 参数
    // * `name` - 列族名称
    // # 返回
    // 如果列族存在，返回列族句柄的引用；否则返回 None
    fn get_column_family(&self, name: &str) -> Option<&rust_rocksdb::ColumnFamily> {
        self.db.cf_handle(name)
    }

    // 列出所有列族名称
    pub fn list_column_families(&self) -> Vec<String> {
        self.column_family_names.clone()
    }

    // 在指定列族中获取值
    // # 参数
    // * `cf_name` - 列族名称
    // * `key` - 键
    pub fn get_cf(&self, cf_name: &str, key: &str) -> Option<Vec<u8>> {
        self.get_column_family(cf_name)
            .and_then(|cf| self.db.get_cf(cf, key.as_bytes()).ok().flatten())
    }

    // 在指定列族中设置值
    // # 参数
    // * `cf_name` - 列族名称
    // * `key` - 键
    // * `value` - 值
    pub fn put_cf(&mut self, cf_name: &str, key: &str, value: Vec<u8>) -> Result<(), RocksDBStoreError> {
        match self.get_column_family(cf_name) {
            Some(cf) => {
                self.db.put_cf(cf, key.as_bytes(), value)
                    .map_err(|e| RocksDBStoreError::from(e))
            }
            None => Err(RocksDBStoreError::new(format!("Column family '{}' not found", cf_name)))
        }
    }

    // 在指定列族中删除值
    // # 参数
    // * `cf_name` - 列族名称
    // * `key` - 键
    pub fn delete_cf(&mut self, cf_name: &str, key: &str) -> Result<(), RocksDBStoreError> {
        match self.get_column_family(cf_name) {
            Some(cf) => {
                self.db.delete_cf(cf, key.as_bytes())
                    .map_err(|e| RocksDBStoreError::from(e))
            }
            None => Err(RocksDBStoreError::new(format!("Column family '{}' not found", cf_name)))
        }
    }
}

impl Snapshot for RocksDBStore {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.get_column_family("default")
            .and_then(|cf| self.db.get_cf(cf, key.as_bytes()).ok().flatten())
    }

    fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

impl Storage for RocksDBStore {
    fn snapshot(&self) -> Box<dyn Snapshot> {
        let snapshot = self.db.snapshot();
        Box::new(RocksDBSnapshot {
            db: Arc::clone(&self.db),
            snapshot: unsafe { std::mem::transmute(snapshot) },
        })
    }

    fn put(&mut self, key: &str, value: Vec<u8>) {
        if let Some(cf) = self.get_column_family("default") {
            let _ = self.db.put_cf(cf, key.as_bytes(), value);
        }
    }

    fn delete(&mut self, key: &str) {
        if let Some(cf) = self.get_column_family("default") {
            let _ = self.db.delete_cf(cf, key.as_bytes());
        }
    }
}

pub struct RocksDBSnapshot {
    db: Arc<DB>,
    snapshot: rust_rocksdb::SnapshotWithThreadMode<'static, DB>,
}

impl Snapshot for RocksDBSnapshot {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        let cf = self.db.cf_handle("default")?;
        let mut read_opts = ReadOptions::default();
        read_opts.set_snapshot(&self.snapshot);
        self.db.get_cf_opt(cf, key.as_bytes(), &read_opts).ok().flatten()
    }

    fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

// RocksDB Fork 实现
// 支持列族的 Fork 功能，可以在不修改原始存储的情况下进行临时修改
pub struct RocksDBFork<'a> {
    storage: &'a RocksDBStore,
    snapshot: Box<dyn Snapshot>,
    changes: HashMap<String, Change>,
    cf_changes: HashMap<String, HashMap<String, Change>>,
}

#[derive(Clone, Debug)]
enum Change {
    Put(Vec<u8>),
    Delete,
}

impl<'a> RocksDBFork<'a> {
    // 创建新的 Fork
    pub fn new(storage: &'a RocksDBStore) -> Self {
        Self {
            storage,
            snapshot: storage.snapshot(),
            changes: HashMap::new(),
            cf_changes: HashMap::new(),
        }
    }

    // 在默认列族中获取值
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        match self.changes.get(key) {
            Some(Change::Put(value)) => Some(value.clone()),
            Some(Change::Delete) => None,
            None => self.snapshot.get(key),
        }
    }

    // 在指定列族中获取值
    pub fn get_cf(&self, cf_name: &str, key: &str) -> Option<Vec<u8>> {
        if let Some(cf_changes) = self.cf_changes.get(cf_name) {
            match cf_changes.get(key) {
                Some(Change::Put(value)) => return Some(value.clone()),
                Some(Change::Delete) => return None,
                None => {}
            }
        }
        self.storage.get_cf(cf_name, key)
    }

    // 在默认列族中设置值
    pub fn put(&mut self, key: &str, value: Vec<u8>) {
        self.changes.insert(key.to_string(), Change::Put(value));
    }

    // 在指定列族中设置值
    pub fn put_cf(&mut self, cf_name: &str, key: &str, value: Vec<u8>) {
        self.cf_changes
            .entry(cf_name.to_string())
            .or_insert_with(HashMap::new)
            .insert(key.to_string(), Change::Put(value));
    }

    // 在默认列族中删除值
    pub fn delete(&mut self, key: &str) {
        self.changes.insert(key.to_string(), Change::Delete);
    }

    // 在指定列族中删除值
    pub fn delete_cf(&mut self, cf_name: &str, key: &str) {
        self.cf_changes
            .entry(cf_name.to_string())
            .or_insert_with(HashMap::new)
            .insert(key.to_string(), Change::Delete);
    }

    // 将 Fork 转换为 Patch
    pub fn into_patch(self) -> RocksDBPatch {
        RocksDBPatch {
            changes: self.changes,
            cf_changes: self.cf_changes,
        }
    }

    // 回滚所有更改
    pub fn rollback(&mut self) {
        self.changes.clear();
        self.cf_changes.clear();
    }
}

// RocksDB Patch
// 包含所有更改的补丁，可以应用到存储
pub struct RocksDBPatch {
    changes: HashMap<String, Change>,
    cf_changes: HashMap<String, HashMap<String, Change>>,
}

impl RocksDBPatch {
    pub fn apply(self, storage: &mut RocksDBStore) -> Result<(), RocksDBStoreError> {
        for (key, change) in self.changes {
            match change {
                Change::Put(value) => storage.put(&key, value),
                Change::Delete => storage.delete(&key),
            }
        }

        for (cf_name, cf_changes) in self.cf_changes {
            for (key, change) in cf_changes {
                match change {
                    Change::Put(value) => storage.put_cf(&cf_name, &key, value)?,
                    Change::Delete => storage.delete_cf(&cf_name, &key)?,
                }
            }
        }

        Ok(())
    }
}

impl RocksDBStore {
    pub fn fork(&self) -> RocksDBFork<'_> {
        RocksDBFork::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_rocksdb_store() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test_db");
        
        let mut storage = RocksDBStore::new(&db_path).unwrap();
        storage.put("key1", b"value1".to_vec());
        storage.put("key2", b"value2".to_vec());
        
        let snapshot = storage.snapshot();
        assert_eq!(snapshot.get("key1"), Some(b"value1".to_vec()));
        assert_eq!(snapshot.get("key2"), Some(b"value2".to_vec()));
        
        storage.put("key1", b"new_value1".to_vec());
        storage.put("key3", b"value3".to_vec());
        storage.delete("key2");
        
        assert_eq!(snapshot.get("key1"), Some(b"value1".to_vec()), "快照应该看到创建时的旧值");
        assert_eq!(snapshot.get("key2"), Some(b"value2".to_vec()), "快照应该看到已删除的键");
        assert_eq!(snapshot.get("key3"), None, "快照不应该看到创建后添加的键");
        
        let new_snapshot = storage.snapshot();
        assert_eq!(new_snapshot.get("key1"), Some(b"new_value1".to_vec()));
        assert_eq!(new_snapshot.get("key2"), None);
        assert_eq!(new_snapshot.get("key3"), Some(b"value3".to_vec()));
    }

    #[test]
    fn test_rocksdb_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("persist_db");
        
        {
            let mut storage = RocksDBStore::new(&db_path).unwrap();
            storage.put("persistent_key", b"persistent_value".to_vec());
        }
        
        let storage = RocksDBStore::new(&db_path).unwrap();
        let snapshot = storage.snapshot();
        assert_eq!(snapshot.get("persistent_key"), Some(b"persistent_value".to_vec()));
    }

    #[test]
    fn test_column_families() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("cf_db");
        
        let mut storage = RocksDBStore::with_column_families(&db_path, &["users", "posts"]).unwrap();
        
        storage.put("default_key", b"default_value".to_vec());
        
        storage.put_cf("users", "user1", b"user1_value".to_vec()).unwrap();
        
        storage.put_cf("posts", "post1", b"post1_value".to_vec()).unwrap();
        
        assert_eq!(storage.get("default_key"), Some(b"default_value".to_vec()));
        assert_eq!(storage.get_cf("users", "user1"), Some(b"user1_value".to_vec()));
        assert_eq!(storage.get_cf("posts", "post1"), Some(b"post1_value".to_vec()));
        
        let cfs = storage.list_column_families();
        assert!(cfs.contains(&"default".to_string()));
        assert!(cfs.contains(&"users".to_string()));
        assert!(cfs.contains(&"posts".to_string()));
    }

    #[test]
    fn test_config() {
        let temp_dir = TempDir::new().unwrap();
        
        // todo!("性能指标监控");
        let db_path1 = temp_dir.path().join("config_db1");
        let config = RocksDBConfig::high_performance();
        let _storage = RocksDBStore::with_config(&db_path1, config, &[]).unwrap();
        
        let db_path2 = temp_dir.path().join("config_db2");
        let config = RocksDBConfig::low_memory();
        let _storage = RocksDBStore::with_config(&db_path2, config, &[]).unwrap();
    }

    #[test]
    fn test_column_family_configs() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("cf_config_db");
        
        let mut db_config = RocksDBConfig::default();
        db_config.max_background_jobs = Some(4);
        db_config.paranoid_checks = Some(true);
        
        // 为不同列族创建不同配置
        let mut cf_configs = HashMap::new();
        
        // users 列族：大缓冲区，LZ4 压缩
        let mut users_cf = ColumnFamilyConfig::default();
        users_cf.write_buffer_size = Some(128 * 1024 * 1024);
        users_cf.compression_type = Some(rust_rocksdb::DBCompressionType::Lz4);
        cf_configs.insert("users".to_string(), users_cf);
        
        // posts 列族：小缓冲区，Snappy 压缩
        let mut posts_cf = ColumnFamilyConfig::default();
        posts_cf.write_buffer_size = Some(32 * 1024 * 1024);
        posts_cf.compression_type = Some(rust_rocksdb::DBCompressionType::Snappy);
        cf_configs.insert("posts".to_string(), posts_cf);
        
        let mut storage = RocksDBStore::with_config_and_cf_configs(
            &db_path,
            db_config,
            &["users", "posts"],
            cf_configs,
        ).unwrap();
        
        storage.put_cf("users", "user1", b"Alice".to_vec()).unwrap();
        storage.put_cf("posts", "post1", b"Hello".to_vec()).unwrap();
        
        assert_eq!(storage.get_cf("users", "user1"), Some(b"Alice".to_vec()));
        assert_eq!(storage.get_cf("posts", "post1"), Some(b"Hello".to_vec()));
    }

    #[test]
    fn test_rocksdb_fork() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("fork_db");
        
        let mut storage = RocksDBStore::new(&db_path).unwrap();
        storage.put("key1", b"value1".to_vec());
        
        let mut fork = storage.fork();
        assert_eq!(fork.get("key1"), Some(b"value1".to_vec()));
        
        fork.put("key1", b"value2".to_vec());
        fork.put("key2", b"value3".to_vec());
        
        let snapshot = storage.snapshot();
        assert_eq!(snapshot.get("key1"), Some(b"value1".to_vec()));
        
        assert_eq!(fork.get("key1"), Some(b"value2".to_vec()));
        assert_eq!(fork.get("key2"), Some(b"value3".to_vec()));
        
        let patch = fork.into_patch();
        patch.apply(&mut storage).unwrap();
        
        let snapshot = storage.snapshot();
        assert_eq!(snapshot.get("key1"), Some(b"value2".to_vec()));
        assert_eq!(snapshot.get("key2"), Some(b"value3".to_vec()));
    }

    #[test]
    fn test_rocksdb_fork_with_column_families() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("fork_cf_db");
        
        let mut storage = RocksDBStore::with_column_families(&db_path, &["users", "posts"]).unwrap();
        storage.put_cf("users", "user1", b"Alice".to_vec()).unwrap();
        storage.put_cf("posts", "post1", b"Hello".to_vec()).unwrap();
        
        let mut fork = storage.fork();
        
        assert_eq!(fork.get_cf("users", "user1"), Some(b"Alice".to_vec()));
        assert_eq!(fork.get_cf("posts", "post1"), Some(b"Hello".to_vec()));
        
        fork.put_cf("users", "user1", b"Bob".to_vec());
        fork.put_cf("users", "user2", b"Charlie".to_vec());
        fork.put_cf("posts", "post1", b"World".to_vec());
        
        assert_eq!(fork.get_cf("users", "user1"), Some(b"Bob".to_vec()));
        assert_eq!(fork.get_cf("users", "user2"), Some(b"Charlie".to_vec()));
        assert_eq!(fork.get_cf("posts", "post1"), Some(b"World".to_vec()));
        
        let _snapshot = storage.snapshot();
        assert_eq!(storage.get_cf("users", "user1"), Some(b"Alice".to_vec()));
        
        let patch = fork.into_patch();
        patch.apply(&mut storage).unwrap();
        
        assert_eq!(storage.get_cf("users", "user1"), Some(b"Bob".to_vec()));
        assert_eq!(storage.get_cf("users", "user2"), Some(b"Charlie".to_vec()));
        assert_eq!(storage.get_cf("posts", "post1"), Some(b"World".to_vec()));
    }
}
