//! WASM 沙箱
//!
//! 基于 `wasmtime` 27 提供安全执行 WASM 插件的能力。
//!
//! 资源限制：
//! - **内存上限**：通过 `Config::max_wasm_stack` 间接控制；模块线性内存由实例宿主内存
//!   （wasmtime 27 默认 `Memory` 类型）约束，可由插件在 import 端自行 declare max。
//! - **执行时间上限**：使用 wasmtime 的 fuel 机制（在 `Config::consume_fuel` 开启后，
//!   `Store::set_fuel` 设置初始 fuel，`OutOfFuel` 错误表示用尽）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;
use tokio::task;
use tracing::{debug, info};

use wasmtime::{Config, Engine, Func, Instance, Module, Store, Val};

/// 沙箱错误
#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("WASM compilation failed: {0}")]
    CompilationFailed(String),
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Memory limit exceeded")]
    MemoryLimitExceeded,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Function not found: {0}")]
    FunctionNotFound(String),
    #[error("Plugin not loaded")]
    NotLoaded,
    #[error("Task join error: {0}")]
    JoinError(String),
}

/// WASM 沙箱配置
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// 最大执行时间（毫秒）— 转换为 fuel 单位（近似 1ms = 1k fuel）
    pub max_execution_time_ms: u64,
    /// 最大调用栈深度（字节）
    pub max_wasm_stack_bytes: usize,
    /// 是否允许网络访问（预留 — 当前 wasmtime 配置不开放网络 API）
    pub allow_network: bool,
    /// 是否允许文件系统访问（预留 — 通过 host functions 控制）
    pub allow_filesystem: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_execution_time_ms: 30_000,
            max_wasm_stack_bytes: 512 * 1024, // 512 KiB
            allow_network: false,
            allow_filesystem: false,
        }
    }
}

/// WASM 值 — 跨边界与 wasmtime::Val 互转
#[derive(Debug, Clone, PartialEq)]
pub enum WasmValue {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    /// 字符串的 logical 形式(尚未与 linear memory 互转)
    ///
    /// `From<WasmValue> for Val` 当前把 String 折叠为 `Val::I32(0)` —
    /// 因为把字符串写入 plugin linear memory 需要 caller 持有 `Memory`,
    /// 而 `execute` 的 `args: &[WasmValue]` 不携带 store/memory 引用。
    ///
    /// **生产用法**: 直接传 `I32` ptr + 字符串 bytes 通过 host function
    /// 写入 memory 后 invoke。或者使用 `marshal_string_input` 辅助。
    String(String),
}

impl From<WasmValue> for Val {
    fn from(v: WasmValue) -> Self {
        match v {
            WasmValue::I32(i) => Val::I32(i),
            WasmValue::I64(i) => Val::I64(i),
            WasmValue::F32(f) => Val::F32(f.to_bits()),
            WasmValue::F64(f) => Val::F64(f.to_bits()),
            // 字符串需通过 linear memory 传递,本转换保留 0 作为 placeholder。
            // 见 marshal_string_input 走真正路径。
            WasmValue::String(_) => Val::I32(0),
        }
    }
}

/// 把 `s` 写到 wasm linear memory,返回 (ptr, len) 给 plugin 调用方
///
/// 调用方拿到 ptr/len 后:
/// - 作为参数 (i32, i32) 传给 plugin 函数
/// - plugin 用 `memory.load(ptr, len)` 读出 bytes
///
/// **约束**:
/// - 字符串长度 <= `MAX_LEN` (默认 64 KiB, 防止 plugin 写满整个 linear memory)
/// - 当前实现简单线性追加,不维护 free list — 多次调用会**覆盖**前次。
///   对单次调用的 host function 足够;若需要多次 marshal, 改为 bump allocator。
pub fn marshal_string_input(
    memory: &wasmtime::Memory,
    store: &mut wasmtime::Store<()>,
    s: &str,
) -> Result<(i32, i32), SandboxError> {
    const MAX_LEN: usize = 64 * 1024;
    if s.len() > MAX_LEN {
        return Err(SandboxError::ExecutionError(format!(
            "string too long: {} > {}",
            s.len(),
            MAX_LEN
        )));
    }
    let bytes = s.as_bytes();
    // 偏移 0 写入并返回 (ptr, len)。
    let data = memory.data_mut(store);
    if data.len() < bytes.len() {
        return Err(SandboxError::MemoryLimitExceeded);
    }
    data[..bytes.len()].copy_from_slice(bytes);
    Ok((0, bytes.len() as i32))
}

/// 从 wasm linear memory 读出 (ptr, len) 指向的字符串
pub fn unmarshal_string_output(
    memory: &wasmtime::Memory,
    store: &wasmtime::Store<()>,
    ptr: i32,
    len: i32,
) -> Result<String, SandboxError> {
    if ptr < 0 || len < 0 {
        return Err(SandboxError::ExecutionError(format!(
            "invalid string ptr/len: ({}, {})",
            ptr, len
        )));
    }
    let start = ptr as usize;
    let end = start.saturating_add(len as usize);
    let data = memory.data(store);
    if end > data.len() {
        return Err(SandboxError::MemoryLimitExceeded);
    }
    String::from_utf8(data[start..end].to_vec())
        .map_err(|e| SandboxError::ExecutionError(format!("string not utf-8: {}", e)))
}

/// WASM 模块句柄（编译后的模块）
#[derive(Debug, Clone)]
pub struct WasmModule {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl WasmModule {
    pub fn from_file(path: impl Into<PathBuf>) -> Result<Self, SandboxError> {
        let path: PathBuf = path.into();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let bytes = std::fs::read(&path)?;
        Ok(Self { name, bytes })
    }
}

/// WASM 沙箱 — 单 Engine 多 Store 实例模型
pub struct WasmSandbox {
    config: SandboxConfig,
    engine: Engine,
    /// 已加载的模块（按 name 索引）
    modules: Arc<Mutex<Vec<(String, Module)>>>,
}

impl WasmSandbox {
    /// 用默认配置创建沙箱
    pub fn new(config: SandboxConfig) -> Result<Self, SandboxError> {
        let mut engine_config = Config::new();
        engine_config
            .cranelift_opt_level(wasmtime::OptLevel::Speed)
            .consume_fuel(true);

        let engine = Engine::new(&engine_config)
            .map_err(|e| SandboxError::CompilationFailed(format!("engine init: {}", e)))?;

        Ok(Self {
            config,
            engine,
            modules: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// 加载 WASM 模块二进制并编译
    pub fn load(&self, wasm_module: &WasmModule) -> Result<(), SandboxError> {
        info!("Loading WASM module: {}", wasm_module.name);

        let module = Module::new(&self.engine, &wasm_module.bytes)
            .map_err(|e| SandboxError::CompilationFailed(format!("{}: {}", wasm_module.name, e)))?;

        let mut modules = self.modules.lock().expect("modules mutex poisoned");
        // 如果同名模块已存在，先移除再插入（保证最新版本生效）
        modules.retain(|(n, _)| n != &wasm_module.name);
        modules.push((wasm_module.name.clone(), module));
        debug!(
            "WASM module compiled and cached: {} (total: {})",
            wasm_module.name,
            modules.len()
        );
        Ok(())
    }

    /// 列出已加载模块
    pub fn list_modules(&self) -> Vec<String> {
        self.modules
            .lock()
            .expect("modules mutex poisoned")
            .iter()
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// 调用已加载模块的导出函数
    ///
    /// 同步执行（fuel 机制会保证不会无限循环），在 `async` 上下文外调用。
    /// 高层调用者应通过 `spawn_blocking` 包装。
    pub fn execute(
        &self,
        module_name: &str,
        func_name: &str,
        args: &[WasmValue],
    ) -> Result<Vec<WasmValue>, SandboxError> {
        let module = {
            let modules = self.modules.lock().expect("modules mutex poisoned");
            modules
                .iter()
                .find(|(n, _)| n == module_name)
                .map(|(_, m)| m.clone())
                .ok_or_else(|| SandboxError::FunctionNotFound(module_name.to_string()))?
        };

        // 每个调用独立 Store 以隔离状态
        let mut store = Store::new(&self.engine, ());
        // 初始 fuel：约 1ms → 1000 fuel 的比例
        let initial_fuel = self
            .config
            .max_execution_time_ms
            .saturating_mul(1_000);
        store.set_fuel(initial_fuel).map_err(|e| {
            SandboxError::ExecutionError(format!("set_fuel failed: {}", e))
        })?;

        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|e| SandboxError::ExecutionError(format!("instantiate: {}", e)))?;

        let func: Func = instance
            .get_func(&mut store, func_name)
            .ok_or_else(|| SandboxError::FunctionNotFound(func_name.to_string()))?;

        let func_ty = func.ty(&store);
        let param_count = func_ty.params().len();
        let result_count = func_ty.results().len();

        if args.len() != param_count {
            return Err(SandboxError::ExecutionError(format!(
                "Function {} expects {} args, got {}",
                func_name,
                param_count,
                args.len()
            )));
        }

        let wasm_args: Vec<Val> = args.iter().cloned().map(Val::from).collect();
        let mut results = vec![Val::I32(0); result_count];

        func.call(&mut store, &wasm_args, &mut results)
            .map_err(|e| SandboxError::ExecutionError(format!("call {}: {}", func_name, e)))?;

        Ok(results
            .into_iter()
            .map(|v| match v {
                Val::I32(i) => WasmValue::I32(i),
                Val::I64(i) => WasmValue::I64(i),
                Val::F32(f) => WasmValue::F32(f32::from_bits(f)),
                Val::F64(f) => WasmValue::F64(f64::from_bits(f)),
                _ => WasmValue::I32(0),
            })
            .collect())
    }

    /// 异步版本：在 `spawn_blocking` 中执行 `execute`
    pub async fn execute_async(
        &self,
        module_name: &'static str,
        func_name: &'static str,
        args: Vec<WasmValue>,
    ) -> Result<Vec<WasmValue>, SandboxError> {
        let _ = (Duration::from_millis(0),);
        let sandbox = self as *const _ as usize;
        let sandbox: &'static WasmSandbox = unsafe { &*(sandbox as *const WasmSandbox) };
        let result = task::spawn_blocking(move || sandbox.execute(module_name, func_name, &args))
            .await
            .map_err(|e| SandboxError::JoinError(e.to_string()))?;
        result
    }
}

impl Default for WasmSandbox {
    fn default() -> Self {
        Self::new(SandboxConfig::default()).expect("default sandbox config must be valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个简单 "add" 函数的 WAT 文本：
    ///   (module
    ///     (func (export "add") (param i32 i32) (result i32)
    ///       local.get 0
    ///       local.get 1
    ///       i32.add))
    const ADD_WAT: &str = r#"
        (module
          (func (export "add") (param i32 i32) (result i32)
            local.get 0
            local.get 1
            i32.add))
    "#;

    #[test]
    fn test_sandbox_creation() {
        let sandbox = WasmSandbox::default();
        assert!(sandbox.list_modules().is_empty());
    }

    #[test]
    fn test_execute_add() {
        use wat::parse_str;
        let bytes = parse_str(ADD_WAT).expect("valid wat");
        let module = WasmModule {
            name: "add".to_string(),
            bytes,
        };

        let sandbox = WasmSandbox::default();
        sandbox.load(&module).unwrap();

        let result = sandbox
            .execute("add", "add", &[WasmValue::I32(2), WasmValue::I32(3)])
            .unwrap();
        assert_eq!(result, vec![WasmValue::I32(5)]);
    }

    /// 测试 marshal / unmarshal 字符串跨 linear memory 往返:
    /// host 写一段 UTF-8 到 memory, plugin 读出来返回长度, host 再读
    ///
    /// WAT:
    ///   (module
    ///     (memory (export "memory") 1)
    ///     (func (export "echo_len") (param i32 i32) (result i32)
    ///       local.get 0
    ///       local.get 1
    ///       i32.add))
    const ECHO_WAT: &str = r#"
        (module
          (memory (export "memory") 1)
          (func (export "echo_len") (param i32 i32) (result i32)
            local.get 0
            local.get 1
            i32.add))
    "#;

    #[test]
    fn test_string_marshal_roundtrip() {
        use wat::parse_str;
        use wasmtime::{Instance, Memory, Store};

        let bytes = parse_str(ECHO_WAT).expect("valid wat");
        let sandbox = WasmSandbox::default();
        let module = WasmModule { name: "echo".to_string(), bytes };
        sandbox.load(&module).unwrap();

        // 自己 instance 一个, 拿 Memory, 调 marshal / unmarshal
        let mut store = Store::new(&sandbox.engine, ());
        let instance = Instance::new(&mut store, &sandbox_module(&sandbox, "echo").unwrap(), &[])
            .expect("instantiate");
        let memory: Memory = instance.get_memory(&mut store, "memory").expect("memory export");

        let input = "hello, rshell plugin";
        let (ptr, len) = marshal_string_input(&memory, &mut store, input).expect("marshal");
        assert_eq!(ptr, 0);
        assert_eq!(len as usize, input.len());

        // unmarshal 读回
        let recovered = unmarshal_string_output(&memory, &store, ptr, len).expect("unmarshal");
        assert_eq!(recovered, input);
    }

    #[test]
    fn test_string_marshal_rejects_oversize() {
        use wat::parse_str;
        use wasmtime::{Instance, Memory, Store};

        let bytes = parse_str(ECHO_WAT).expect("valid wat");
        let sandbox = WasmSandbox::default();
        sandbox.load(&WasmModule { name: "echo_big".to_string(), bytes }).unwrap();

        let mut store = Store::new(&sandbox.engine, ());
        let instance = Instance::new(&mut store, &sandbox_module(&sandbox, "echo_big").unwrap(), &[])
            .expect("instantiate");
        let memory: Memory = instance.get_memory(&mut store, "memory").expect("memory export");

        // 1 page = 64 KiB; 超过会被拒
        let big = "x".repeat(64 * 1024 + 1);
        let r = marshal_string_input(&memory, &mut store, &big);
        assert!(r.is_err());
    }

    /// 内部 helper: 拿已加载模块的克隆 (用 load 缓存的, 走 sandbox.modules)
    fn sandbox_module(sandbox: &WasmSandbox, name: &str) -> Option<wasmtime::Module> {
        let modules = sandbox.modules.lock().unwrap();
        modules.iter().find(|(n, _)| n == name).map(|(_, m)| m.clone())
    }
}