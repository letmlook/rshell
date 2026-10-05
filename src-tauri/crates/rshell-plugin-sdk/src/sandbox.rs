//! WASM 沙箱
//!
//! 基于 `wasmtime` 27 提供安全执行 WASM 插件的能力。
//!
//! 资源限制（**每一条都由代码真正强制**，不再是"声明了但没人读"的字段）：
//! - **内存上限**：`execute` 每次调用新建 Store 时挂 `StoreLimits`
//!   （`store.limiter`），单块线性内存最多增长到 `SandboxConfig::max_wasm_memory_bytes`。
//!   没有它，guest 可以把线性内存涨到自身声明的上限（wasm32 最高 4 GiB）把宿主进程撑爆。
//! - **调用栈上限**：`Config::max_wasm_stack` = `SandboxConfig::max_wasm_stack_bytes`，
//!   引擎级生效，栈溢出直接 trap。
//! - **执行时间上限**：wasmtime 的 fuel 机制（`Config::consume_fuel` 开启后，
//!   `Store::set_fuel` 设置初始 fuel，`OutOfFuel` 错误表示用尽）。
//! - **网络 / 文件系统**：**结构性关闭**——`Instance::new` 的 imports 恒为 `&[]`，
//!   guest 拿不到任何 host function。`allow_network` / `allow_filesystem` 为 `true`
//!   时 `WasmSandbox::new` 直接返回错误，而不是"配置被忽略"后静默按关闭执行。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use thiserror::Error;
use tokio::task;
use tracing::{debug, info};

use wasmtime::{
    Config, Engine, Func, Instance, Module, Store, StoreLimits, StoreLimitsBuilder, Val,
};

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
    #[error("Sandbox configuration rejected: {0}")]
    ConfigRejected(String),
}

/// WASM 沙箱配置
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// 最大执行时间（毫秒）— 转换为 fuel 单位（近似 1ms = 1k fuel）
    pub max_execution_time_ms: u64,
    /// 单块线性内存的最大字节数（`StoreLimits::memory_size`）
    pub max_wasm_memory_bytes: usize,
    /// 最大调用栈深度（字节）— 映射到 `Config::max_wasm_stack`
    pub max_wasm_stack_bytes: usize,
    /// 是否允许网络访问。guest 拿不到任何 host import，本字段为 `true` 时
    /// `WasmSandbox::new` 报错而不是被忽略。
    pub allow_network: bool,
    /// 是否允许文件系统访问。能力同样来自 host import（当前一个都没有），
    /// 本字段为 `true` 时 `WasmSandbox::new` 报错而不是被忽略。
    pub allow_filesystem: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_execution_time_ms: 30_000,
            // 64 MiB：远低于 wasm32 的 4 GiB 上限，够脚本型插件用；
            // 不设限则一次 memory.grow 就能把宿主进程 OOM 掉（R3-15）
            max_wasm_memory_bytes: 64 * 1024 * 1024,
            max_wasm_stack_bytes: 512 * 1024, // 512 KiB，与 wasmtime 默认一致
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
    /// 把字符串写入 plugin linear memory 需要 caller 持有 `Memory`,
    /// 而 `execute` 的 `args: &[WasmValue]` 不携带 store/memory 引用,
    /// 因此**不能**把 `String` 直接作为函数参数传入:`execute` 对
    /// `String` 参数显式返回错误(不再静默折叠为 `Val::I32(0)`)。
    ///
    /// **生产用法**: 直接传 `I32` ptr + 字符串 bytes 通过 host function
    /// 写入 memory 后 invoke。或者使用 `marshal_string_input` 辅助。
    String(String),
}

/// `WasmValue` → wasmtime `Val` 的窄化转换。
///
/// 字符串需经由 linear memory 传递(见 `marshal_string_input`),
/// 在字符串序列化实现之前,`String` 参数显式报错,绝不静默折叠。
fn wasm_value_to_val(v: WasmValue) -> Result<Val, SandboxError> {
    match v {
        WasmValue::I32(i) => Ok(Val::I32(i)),
        WasmValue::I64(i) => Ok(Val::I64(i)),
        WasmValue::F32(f) => Ok(Val::F32(f.to_bits())),
        WasmValue::F64(f) => Ok(Val::F64(f.to_bits())),
        WasmValue::String(_) => Err(SandboxError::ExecutionError(
            "String arguments are not supported yet: marshal the string into linear \
             memory via marshal_string_input and pass (ptr, len) as I32 instead"
                .to_string(),
        )),
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
    /// 用给定配置创建沙箱
    ///
    /// 选 (a) 方案：把声明的限流字段真正接进 wasmtime，而不是删字段改注释。
    /// `wasmtime` 已是本 crate 的直接依赖且带 `cranelift` feature（`runtime`
    /// 是默认 feature），`StoreLimits` / `Config::max_wasm_stack` 都不需要新增依赖。
    pub fn new(config: SandboxConfig) -> Result<Self, SandboxError> {
        // 能力开关必须"要么真的生效、要么明确报错"：当前 guest 一个 host import
        // 都拿不到（`Instance::new(..., &[])`），网络/文件系统在结构上就不存在。
        // 放任 `true` 被忽略等于"配置静默成功"，与 CLAUDE.md 的验收口径冲突。
        if config.allow_network {
            return Err(SandboxError::ConfigRejected(
                "allow_network is not implemented: guests receive no host imports at all \
                 (Instance::new is called with an empty import list)"
                    .to_string(),
            ));
        }
        if config.allow_filesystem {
            return Err(SandboxError::ConfigRejected(
                "allow_filesystem is not implemented: guests receive no host imports at all \
                 (Instance::new is called with an empty import list)"
                    .to_string(),
            ));
        }
        if config.max_wasm_memory_bytes == 0 || config.max_wasm_stack_bytes == 0 {
            return Err(SandboxError::ConfigRejected(
                "max_wasm_memory_bytes and max_wasm_stack_bytes must be greater than 0 \
                 (0 would silently disable the limit)"
                    .to_string(),
            ));
        }

        let mut engine_config = Config::new();
        engine_config
            .cranelift_opt_level(wasmtime::OptLevel::Speed)
            .consume_fuel(true)
            // 声明的栈上限真正落到引擎：超过即 trap（wasmtime 默认同样是 512 KiB）
            .max_wasm_stack(config.max_wasm_stack_bytes);

        let engine = Engine::new(&engine_config)
            .map_err(|e| SandboxError::CompilationFailed(format!("engine init: {}", e)))?;

        Ok(Self {
            config,
            engine,
            modules: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// 为一次调用构造带资源上限的 Store。
    ///
    /// `StoreLimits` 挂在 Store 的 data 上（不是局部变量）：`store.limiter` 要求
    /// 闭包返回 `&mut dyn ResourceLimiter`，引用必须活得和 Store 一样久。
    /// 每次调用一个独立 Store，limits 也因此天然按调用隔离。
    fn new_limited_store(&self) -> Store<StoreLimits> {
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.max_wasm_memory_bytes)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|state| state);
        store
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

    /// Remove a compiled module when its plugin is unloaded.
    pub fn unload(&self, name: &str) {
        self.modules
            .lock()
            .expect("modules mutex poisoned")
            .retain(|(module_name, _)| module_name != name);
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

        // 每个调用独立 Store 以隔离状态，并挂上内存上限（R3-15）
        let mut store = self.new_limited_store();
        // 初始 fuel：约 1ms → 1000 fuel 的比例
        let initial_fuel = self.config.max_execution_time_ms.saturating_mul(1_000);
        store
            .set_fuel(initial_fuel)
            .map_err(|e| SandboxError::ExecutionError(format!("set_fuel failed: {}", e)))?;

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

        let wasm_args: Vec<Val> = args
            .iter()
            .cloned()
            .map(wasm_value_to_val)
            .collect::<Result<Vec<_>, _>>()?;
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
    ///
    /// 通过把 `Arc<WasmSandbox>` clone 进闭包保证沙箱存活至任务结束，
    /// 不再持有 `&self` 裸指针，插件 unload 与执行之间不存在
    /// use-after-free 竞态。
    pub async fn execute_async(
        self: Arc<Self>,
        module_name: &str,
        func_name: &str,
        args: Vec<WasmValue>,
    ) -> Result<Vec<WasmValue>, SandboxError> {
        let module_name = module_name.to_string();
        let func_name = func_name.to_string();
        let result = task::spawn_blocking(move || self.execute(&module_name, &func_name, &args))
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
        use wasmtime::{Instance, Memory, Store};
        use wat::parse_str;

        let bytes = parse_str(ECHO_WAT).expect("valid wat");
        let sandbox = WasmSandbox::default();
        let module = WasmModule {
            name: "echo".to_string(),
            bytes,
        };
        sandbox.load(&module).unwrap();

        // 自己 instance 一个, 拿 Memory, 调 marshal / unmarshal
        let mut store = Store::new(&sandbox.engine, ());
        let instance = Instance::new(&mut store, &sandbox_module(&sandbox, "echo").unwrap(), &[])
            .expect("instantiate");
        let memory: Memory = instance
            .get_memory(&mut store, "memory")
            .expect("memory export");

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
        use wasmtime::{Instance, Memory, Store};
        use wat::parse_str;

        let bytes = parse_str(ECHO_WAT).expect("valid wat");
        let sandbox = WasmSandbox::default();
        sandbox
            .load(&WasmModule {
                name: "echo_big".to_string(),
                bytes,
            })
            .unwrap();

        let mut store = Store::new(&sandbox.engine, ());
        let instance = Instance::new(
            &mut store,
            &sandbox_module(&sandbox, "echo_big").unwrap(),
            &[],
        )
        .expect("instantiate");
        let memory: Memory = instance
            .get_memory(&mut store, "memory")
            .expect("memory export");

        // 1 page = 64 KiB; 超过会被拒
        let big = "x".repeat(64 * 1024 + 1);
        let r = marshal_string_input(&memory, &mut store, &big);
        assert!(r.is_err());
    }

    /// 内部 helper: 拿已加载模块的克隆 (用 load 缓存的, 走 sandbox.modules)
    fn sandbox_module(sandbox: &WasmSandbox, name: &str) -> Option<wasmtime::Module> {
        let modules = sandbox.modules.lock().unwrap();
        modules
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, m)| m.clone())
    }

    /// R3-15 回归：声明了超过上限的线性内存的模块必须被 limiter 拒绝。
    /// 旧实现 `Store::new(&engine, ())` 不挂任何 ResourceLimiter，
    /// 128 MiB 的 min memory 会被照常实例化，guest 可以一路涨到 4 GiB 把宿主撑爆。
    #[test]
    fn memory_limit_refuses_oversized_linear_memory() {
        use wat::parse_str;
        // min 2048 page = 128 MiB > 默认上限 64 MiB；max 4096 page = 256 MiB
        let wat = r#"
        (module
          (memory (export "memory") 2048 4096)
          (func (export "noop")))
        "#;
        let sandbox = WasmSandbox::default();
        sandbox
            .load(&WasmModule {
                name: "greedy".to_string(),
                bytes: parse_str(wat).expect("valid wat"),
            })
            .unwrap();

        let err = sandbox
            .execute("greedy", "noop", &[])
            .expect_err("超过 max_wasm_memory_bytes 的模块必须被拒绝");
        let message = err.to_string();
        assert!(
            message.to_lowercase().contains("memory"),
            "错误信息应指向内存上限，实际：{}",
            message
        );
    }

    /// R3-15 回归：内存上限在运行期也生效——`memory.grow` 越界返回 -1，
    /// 而不是让 guest 真的把线性内存涨上去。
    #[test]
    fn memory_grow_beyond_limit_is_refused() {
        use wat::parse_str;
        let wat = r#"
        (module
          (memory (export "memory") 1 4096)
          (func (export "grow") (param i32) (result i32)
            local.get 0
            memory.grow))
        "#;
        let sandbox = WasmSandbox::default();
        let cap_pages = (SandboxConfig::default().max_wasm_memory_bytes / (64 * 1024)) as i32;
        sandbox
            .load(&WasmModule {
                name: "grower".to_string(),
                bytes: parse_str(wat).expect("valid wat"),
            })
            .unwrap();

        // 涨到上限以内：允许（memory.grow 返回的是**增长前**的页数，不是 0）
        let ok = sandbox
            .execute("grower", "grow", &[WasmValue::I32(cap_pages - 1)])
            .unwrap();
        assert_ne!(
            ok,
            vec![WasmValue::I32(-1)],
            "上限以内的增长应成功，实际：{:?}",
            ok
        );

        // 再涨到声明上限以上：被 limiter 拒绝，wasm 语义下返回 -1
        let denied = sandbox
            .execute("grower", "grow", &[WasmValue::I32(4096)])
            .unwrap();
        assert_eq!(
            denied,
            vec![WasmValue::I32(-1)],
            "越过 max_wasm_memory_bytes 的 memory.grow 必须失败"
        );
    }

    /// R3-15 回归：能力开关不能"被忽略后静默按关闭执行"。
    /// guest 一个 host import 都拿不到，宣称允许网络/文件系统就是撒谎。
    #[test]
    fn capability_toggles_are_rejected_instead_of_ignored() {
        let network = SandboxConfig {
            allow_network: true,
            ..SandboxConfig::default()
        };
        assert!(
            WasmSandbox::new(network).is_err(),
            "allow_network 尚未实现，必须报错而不是被忽略"
        );

        let fs = SandboxConfig {
            allow_filesystem: true,
            ..SandboxConfig::default()
        };
        assert!(
            WasmSandbox::new(fs).is_err(),
            "allow_filesystem 尚未实现，必须报错而不是被忽略"
        );
    }

    /// R3-15 回归：`max_wasm_stack_bytes` 必须真正接进引擎配置。
    ///
    /// 同一个有界递归模块：栈配 8 MiB 时 20_000 层跑完，栈用 wasmtime 默认的
    /// 512 KiB 时 trap。只有字段真的进了 `Config::max_wasm_stack` 才有这个差别
    /// ——否则两边行为一致（要么都成功、要么都 trap），测试不成立。
    #[test]
    fn max_wasm_stack_bytes_is_applied_to_the_engine() {
        use wat::parse_str;
        let wat = r#"
        (module
          (global $n (mut i32) (i32.const 0))
          (func $rec (export "recurse") (param i32) (result i32)
            (if (i32.lt_u (global.get $n) (local.get 0))
              (then
                (global.set $n (i32.add (global.get $n) (i32.const 1)))
                (drop (call $rec (local.get 0)))))
            (i32.const 1)))
        "#;
        let bytes = parse_str(wat).expect("valid wat");
        let depth = 20_000i32;

        let wide = SandboxConfig {
            max_wasm_stack_bytes: 8 * 1024 * 1024,
            ..SandboxConfig::default()
        };
        let wide_sandbox = WasmSandbox::new(wide).expect("valid config");
        wide_sandbox
            .load(&WasmModule {
                name: "stackhog_wide".to_string(),
                bytes: bytes.clone(),
            })
            .unwrap();
        assert!(
            wide_sandbox
                .execute("stackhog_wide", "recurse", &[WasmValue::I32(depth)])
                .is_ok(),
            "配置 8 MiB 栈时 {} 层递归应跑完（栈上限字段必须真的生效）",
            depth
        );

        // 对照组：默认 512 KiB 栈必须 trap——证明上一条不是因为"压根没限制"
        let narrow_sandbox = WasmSandbox::default();
        assert_eq!(
            narrow_sandbox.config.max_wasm_stack_bytes,
            512 * 1024,
            "对照组用的就是 wasmtime 默认栈大小"
        );
        narrow_sandbox
            .load(&WasmModule {
                name: "stackhog_narrow".to_string(),
                bytes,
            })
            .unwrap();
        assert!(
            narrow_sandbox
                .execute("stackhog_narrow", "recurse", &[WasmValue::I32(depth)])
                .is_err(),
            "默认 {} 栈跑 {} 层递归必须 trap",
            narrow_sandbox.config.max_wasm_stack_bytes,
            depth
        );
    }
}
