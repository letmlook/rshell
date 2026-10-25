// src/ipc/client.ts - Tauri invoke 包装
//
// 所有 AppCommand 变体通过 invoke() 发到后端。
// 每个 helper 对应 commands.rs 里的 #[tauri::command] 函数。

import { invoke } from "@tauri-apps/api/core";
import type {
  AppCommand,
  ConflictPolicy,
  ConnectionState,
  Folder,
  HostKeyEntry,
  PathBuf,
  PortForwardRule,
  PluginInfo,
  ActiveTunnelInfo,
  PendingTunnelInfo,
  RemoteFileEntry,
  TransferTaskInfo,
  Protocol,
  QuickCommand,
  SessionConfig,
  SshKeyType,
  TerminalColorScheme,
  ThemeColors,
  Trigger,
  TrustHostKeyDecision,
  Uuid,
  ComposeTarget,
  CredentialUpdate,
  SessionCredential,
  SessionLoadIssue,
  IpcErrorKind,
  SshKeyInfo,
} from "./types";

// ===== 通用 helper =====

/** 显式把 AppCommand 转成 (command_name, payload) 元组 */
function commandToArgs(cmd: AppCommand): [string, Record<string, unknown>] {
  const entries = Object.entries(cmd);
  if (entries.length !== 1) {
    throw new Error(`AppCommand must be exactly one variant, got ${entries.length}`);
  }
  const [name, payload] = entries[0];
  return [commandName(name), payload as Record<string, unknown>];
}

/** 把 Rust 风格 PascalCase 变体名转成 Tauri 命令名(snake_case) */
function commandName(variant: string): string {
  // 例如 "ConnectSession" -> "connect_session"
  return variant
    .replace(/([A-Z])/g, "_$1")
    .toLowerCase()
    .replace(/^_/, "");
}

async function call<T = unknown>(cmd: AppCommand): Promise<T> {
  const [name, payload] = commandToArgs(cmd);
  try {
    return await invoke<T>(name, payload);
  } catch (e) {
    if (isIpcErrorShape(e)) throw new IpcCallError(e);
    throw e;
  }
}

// ===== 后端错误形状转换（R2-02） =====
//
// Tauri 2 对实现了 Serialize 的 IpcError（src-tauri/src/error.rs）走
// InvokeError(serde_json::Value) 整体序列化：前端 catch 到的是
// { kind, message, session_id } 对象而非字符串，直接 String(e) 会显示成
// "[object Object]"。这里在唯一入口 call() 统一转回可读 Error：
// message 作展示文案，kind / session_id 挂在 error 属性上供调用方分支。

/** 后端 IpcError 经 Tauri IPC 序列化后抵达前端的形状 */
export interface IpcErrorShape {
  kind: IpcErrorKind;
  message: string;
  session_id?: string | null;
}

const IPC_ERROR_KINDS = new Set<IpcErrorKind>([
  "not_found",
  "auth_failed",
  "host_key_mismatch",
  "credential_missing",
  "credential_inaccessible",
  "credential_save_failed",
  "credential_migration_failed",
  "host_key_trust_persistence_failed",
  "connection",
  "io",
  "permission",
  "outcome_mismatch",
  "internal",
  "storage",
  "target_exists",
  "terminal_recovery_required",
]);

export function isIpcErrorShape(value: unknown): value is IpcErrorShape {
  return (
    typeof value === "object" &&
    value !== null &&
    IPC_ERROR_KINDS.has((value as { kind?: unknown }).kind as IpcErrorKind) &&
    typeof (value as { message?: unknown }).message === "string"
  );
}

/** 可读的后端错误：String(e) 显示 message 而非 "[object Object]" */
export class IpcCallError extends Error {
  /** 稳定的机器可读判别串（not_found / connection / ...），供前端分支 */
  readonly kind: IpcErrorKind;
  /** 可选会话 id，便于把错误挂到正确的会话行 */
  readonly session_id: string | null;
  constructor(shape: IpcErrorShape) {
    super(shape.message);
    this.name = "IpcError";
    this.kind = shape.kind;
    this.session_id = shape.session_id ?? null;
  }
}

/** 任意 rejection 的可读文案：IpcError 对象取 message，其余维持原语义
 * （Error 取 message、字符串原样），供绕过 call() 的终端 Channel 路径复用 */
export function ipcErrorMessage(e: unknown): string {
  if (isIpcErrorShape(e)) return e.message;
  return e instanceof Error ? e.message : String(e);
}

// ===== 会话 =====
//
// 切片 1.2 起：后端 `list_sessions` 改用 §3.4 直接返回 `Vec<SessionConfig>`,
// 不再包 `{ sessions: [...] }`。`create_session` 直接返回 `Uuid`(新会话 id)。
export const listSessions = () => call<SessionConfig[]>({ ListSessions: null });
export const listSessionLoadIssues = () => call<SessionLoadIssue[]>({ ListSessionLoadIssues: null });
export const retrySessionLoad = () => call({ RetrySessionLoad: null });

export const createSession = (config: SessionConfig, credential: SessionCredential | null) =>
  call<Uuid>({ CreateSession: { config, credential } });

export const updateSession = (id: Uuid, config: SessionConfig, credential: CredentialUpdate) =>
  call({ UpdateSession: { id, config, credential } });

export const deleteSession = (id: Uuid) => call({ DeleteSession: { id } });

export const connectSession = (session_id: Uuid) =>
  call({ ConnectSession: { session_id } });

export const disconnectSession = (session_id: Uuid) =>
  call({ DisconnectSession: { session_id } });

// ===== 终端 =====
// terminal_id 决定操作落在哪个 pty：每个标签一个独立 pty（独立 shell）。
// 传 undefined = 该连接的主 pty（连接建立时创建的首标签）。

export const sendInput = (session_id: Uuid, data: Uint8Array | number[], terminal_id?: Uuid) =>
  call({ SendInput: { session_id, terminal_id: terminal_id ?? null, data: Array.from(data) } });

export const resizeTerminal = (session_id: Uuid, cols: number, rows: number, terminal_id?: Uuid) =>
  call({ ResizeTerminal: { session_id, terminal_id: terminal_id ?? null, cols, rows } });

/** 在已连接的会话上另开一个 pty（= 一个独立标签会话） */
export const openTerminal = (session_id: Uuid, terminal_id: Uuid, cols: number, rows: number) =>
  call({ OpenTerminal: { session_id, terminal_id, cols, rows } });

/** 关闭一个 pty（标签关闭）；断开连接时无需调用 */
export const closeTerminal = (session_id: Uuid, terminal_id: Uuid) =>
  call({ CloseTerminal: { session_id, terminal_id } });

// 切片 2.2 删除（设计 §5）：CopySelection 上移到前端 ——
// xterm.js 自持选区,后端不再发 ClipboardCopy 事件,前端用 navigator.clipboard
// 直接写入（不引入 clipboard 插件依赖）。

export interface ThemeInfo {
  current_theme: string;
  current_scheme: string;
  current_colors: ThemeColors;
  current_palette: TerminalColorScheme;
  available_themes: string[];
  available_schemes: string[];
}

// ===== 文件传输 =====

export const enqueueUpload = (local: PathBuf, remote: string, session_id: Uuid, conflict?: ConflictPolicy) =>
  call({ EnqueueUpload: { local, remote, session_id, conflict: conflict ?? "Fail" } });

export const enqueueDownload = (remote: string, local: PathBuf, session_id: Uuid, conflict?: ConflictPolicy) =>
  call({ EnqueueDownload: { remote, local, session_id, conflict: conflict ?? "Fail" } });

export const pauseTransfer = (task_id: Uuid) => call({ PauseTransfer: { task_id } });
export const resumeTransfer = (task_id: Uuid) => call({ ResumeTransfer: { task_id } });
export const cancelTransfer = (task_id: Uuid) => call({ CancelTransfer: { task_id } });
/** 从队列移除终态条目；不删除已传输文件。仅终态任务可移除 */
export const removeTransfer = (task_id: Uuid) => call({ RemoveTransfer: { task_id } });
/**
 * R2-T2：从零重试一个**终态**传输任务 —— 创建新 task id 并入队。
 *
 * 故意不复用 `EnqueueUpload` / `EnqueueDownload` 的「hidden flag」：
 * retry 的语义与首次入队不同（强制 `Fail` 策略、跳过预检的存在性检查、
 * 原任务保留其终态），单独命令更安全。
 *
 * 非终态任务会被拒为 `InvalidState`，需先取消再重试。
 */
export const retryTransfer = (task_id: Uuid) =>
  call<Uuid>({ RetryTransfer: { task_id } });

export const browseRemoteDir = (session_id: Uuid, path: string) =>
  call<{ path: string; entries: RemoteFileEntry[] }>({ BrowseRemoteDir: { session_id, path } });

/** 远端用户工作目录（登录后默认所在目录）；后端解析失败时回退 "/" */
export const getRemoteHomeDir = (session_id: Uuid) =>
  call<string>({ GetRemoteHomeDir: { session_id } });
export const createRemoteDirectory = (session_id: Uuid, path: string) =>
  call<void>({ CreateRemoteDirectory: { session_id, path } });
export const deleteRemoteEntry = (session_id: Uuid, path: string) =>
  call<void>({ DeleteRemoteEntry: { session_id, path } });
export const listTransfers = () => call<TransferTaskInfo[]>({ ListTransfers: null });

// ===== 隧道 =====

export const createTunnel = (session_id: Uuid, rule: PortForwardRule) =>
  call({ CreateTunnel: { session_id, rule } });
export const closeTunnel = (tunnel_id: Uuid) => call({ CloseTunnel: { tunnel_id } });
export const listTunnels = () => call<ActiveTunnelInfo[]>({ ListTunnels: null });
export const listPendingTunnels = () => call<PendingTunnelInfo>({ ListPendingTunnels: null });
export const restoreTunnel = (session_id: Uuid, rule: PortForwardRule) =>
  call({ RestoreTunnel: { session_id, rule } });
export const suspendTunnel = (tunnel_id: Uuid) => call({ SuspendTunnel: { tunnel_id } });
export const resumeTunnel = (tunnel_id: Uuid) => call({ ResumeTunnel: { tunnel_id } });

// ===== 串口 =====

/**
 * 列出当前系统可用的串口设备。
 *
 * 用户不该靠手打 `/dev/cu.usbserial-XXXX`——插拔一次名字就变。
 */
export const listSerialPorts = () => call<string[]>({ ListSerialPorts: null });

// ===== 连接分组 =====

/** 列出全部分组（按名称排序） */
export const listFolders = () => call<Folder[]>({ ListFolders: null });
/** 新建分组，返回新分组 id */
export const createFolder = (name: string) => call<Uuid>({ CreateFolder: { name } });
/** 重命名分组 */
export const renameFolder = (folder_id: Uuid, name: string) =>
  call({ RenameFolder: { folder_id, name } });
/** 删除分组：组内的连接回到「默认」，不会被删除 */
export const deleteFolder = (folder_id: Uuid) => call({ DeleteFolder: { folder_id } });

// ===== 快速命令 =====

export const executeQuickCommand = (command_id: Uuid, target_sessions: Uuid[]) =>
  call({ ExecuteQuickCommand: { command_id, target_sessions } });
export const createQuickCommand = (command: QuickCommand) =>
  call({ CreateQuickCommand: { command } });
export const deleteQuickCommand = (command_id: Uuid) =>
  call({ DeleteQuickCommand: { command_id } });
export const listQuickCommands = () => call<QuickCommand[]>({ ListQuickCommands: null });

// ===== 触发器 =====

export const createTrigger = (trigger: Trigger) => call({ CreateTrigger: { trigger } });
export const deleteTrigger = (trigger_id: Uuid) => call({ DeleteTrigger: { trigger_id } });
export const toggleTrigger = (trigger_id: Uuid) => call({ ToggleTrigger: { trigger_id } });
export const listTriggers = () => call<Trigger[]>({ ListTriggers: null });

// ===== 撰写窗格 =====

/**
 * 发送撰写窗格文本到目标会话。
 *
 * `activeSessionId` 只在 target 为 `CurrentSession` 时有意义：后端用它把
 * 「当前会话」解析成真实会话 id，缺省时该目标会以 `No active session` 报错
 * （此前 dispatcher 把这个参数写死为 None，导致「发送到当前会话」永远失败）。
 */
export const sendComposeText = (
  content: string,
  target: ComposeTarget,
  activeSessionId?: Uuid | null,
) =>
  call({
    SendComposeText: {
      content,
      target,
      active_session_id: activeSessionId ?? null,
    },
  });

// ===== 脚本 =====

export const executeScript = (code: string, session_id: Uuid) =>
  call({ ExecuteScript: { code, session_id } });

// ===== 同步输入 =====

/**
 * 设置同步输入会话组。传空数组表示关闭同步输入。
 *
 * 开启后，任意终端的输入会**同时**发到这组会话（源会话本身不重复发送）。
 */
export const toggleSyncInput = (session_ids: Uuid[]) =>
  call({ ToggleSyncInput: { session_ids } });
/** 查询当前同步输入会话组；空数组表示未开启 */
export const getSyncInputSessions = () =>
  call<Uuid[]>({ GetSyncInputSessions: null });

// ===== 密钥管理 =====

export const generateSshKey = (name: string, key_type: SshKeyType, passphrase: string | null) =>
  call({ GenerateSshKey: { name, key_type, passphrase } });
export const importPrivateKey = (path: PathBuf, passphrase: string | null) =>
  call({ ImportPrivateKey: { path, passphrase } });
export const deleteSshKey = (key_id: Uuid) => call({ DeleteSshKey: { key_id } });
export const exportPublicKey = (key_id: Uuid) => call({ ExportPublicKey: { key_id } });
// 切片 3 修正：ListKeys 直接返回数组（切片 2.2 死循环修复后契约对齐）
export const listKeys = () => call<SshKeyInfo[]>({ ListKeys: null });

// ===== 主密码 =====
//
// 主密码只用于应用锁定/验证，不是凭据保险库，也不加密钥匙串。
// 派生密钥永不落盘：后端持久化的只有盐与加密后的验证令牌，
// 因此重启后状态是 Locked，必须重新验证才能解锁。

export type MasterPasswordState = "NotSet" | "Locked" | "Unlocked";

export const setupMasterPassword = (password: string) =>
  call({ SetupMasterPassword: { password } });
// 切片 3 修正：VerifyMasterPassword 直接返回 bool（切片 2.2 CommandOutcome::Verified）
export const verifyMasterPassword = (password: string) =>
  call<boolean>({ VerifyMasterPassword: { password } });
export const changeMasterPassword = (old_password: string, new_password: string) =>
  call({ ChangeMasterPassword: { old_password, new_password } });
/** 停用主密码：必须提供当前密码，后端据此校验后才删除持久化文件 */
export const disableMasterPassword = (password: string) =>
  call({ DisableMasterPassword: { password } });
/** 立即锁定：清除内存中的派生密钥，需重新验证解锁 */
export const lockMasterPassword = () => call({ LockMasterPassword: null });
export const getMasterPasswordState = () =>
  call<MasterPasswordState>({ GetMasterPasswordState: null });

// ===== 主机密钥 =====

// 仅 TrustPermanent 会持久化写入 known_hosts;Reject/TrustOnce 被后端拒绝并返回错误(PROB-06)
export const trustHostKey = (
  host: string,
  port: number,
  key_type: string,
  public_key_blob: string,
  decision: TrustHostKeyDecision,
) =>
  call({
    TrustHostKey: { host, port, key_type, public_key_blob, decision },
  });
export const decideHostKey = (decision_id: Uuid, accept: boolean, permanent: boolean) =>
  call({ DecideHostKey: { decision_id, accept, permanent } });
export const cancelHostKey = (decision_id: Uuid) =>
  call({ CancelHostKey: { decision_id } });
export const deleteHostKey = (host: string, port: number) =>
  call({ DeleteHostKey: { host, port } });
/** 列出已信任的主机密钥；删除前必须先能看到条目 */
export const listHostKeys = () => call<HostKeyEntry[]>({ ListHostKeys: null });

// ===== 主题 =====

export const setAppTheme = (theme_name: string) => call({ SetAppTheme: { theme_name } });
export const setTerminalColorScheme = (scheme_name: string) =>
  call({ SetTerminalColorScheme: { scheme_name } });
export const importColorScheme = (scheme: TerminalColorScheme) =>
  call({ ImportColorScheme: { scheme } });
export const listThemes = () =>
  call<ThemeInfo>({ ListThemes: null });

// 切片 4：decideHostKey —— 已在下方"主机密钥"分区声明（line 196）

// ===== 插件 =====

export const scanPlugins = () => call({ ScanPlugins: null });
export const loadPlugin = (plugin_id: string) => call({ LoadPlugin: { plugin_id } });
export const unloadPlugin = (plugin_id: string) => call({ UnloadPlugin: { plugin_id } });
export const listPlugins = () => call<PluginInfo[]>({ ListPlugins: null });

// 重新导出类型方便使用方
export type { ConnectionState, Protocol };
