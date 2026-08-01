// src/ipc/client.ts - Tauri invoke 包装
//
// 所有 AppCommand 变体通过 invoke() 发到后端。
// 每个 helper 对应 commands.rs 里的 #[tauri::command] 函数。

import { invoke } from "@tauri-apps/api/core";
import type {
  AppCommand,
  ConnectionState,
  PathBuf,
  PortForwardRule,
  Protocol,
  ProtocolType,
  QuickCommand,
  SerialConfig,
  SessionConfig,
  SshKeyType,
  TelnetConfig,
  TerminalColorScheme,
  Trigger,
  TrustHostKeyDecision,
  Uuid,
  ComposeTarget,
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
  return invoke<T>(name, payload);
}

// ===== 会话 =====
//
// 切片 1.2 起：后端 `list_sessions` 改用 §3.4 直接返回 `Vec<SessionConfig>`,
// 不再包 `{ sessions: [...] }`。`create_session` 直接返回 `Uuid`(新会话 id)。
export const listSessions = () => call<SessionConfig[]>({ ListSessions: null });

export const createSession = (config: SessionConfig) =>
  call<Uuid>({ CreateSession: { config } });

export const updateSession = (id: Uuid, config: SessionConfig) =>
  call({ UpdateSession: { id, config } });

export const deleteSession = (id: Uuid) => call({ DeleteSession: { id } });

export const connectSession = (session_id: Uuid) =>
  call({ ConnectSession: { session_id } });

export const disconnectSession = (session_id: Uuid) =>
  call({ DisconnectSession: { session_id } });

// ===== 终端 =====

export const sendInput = (session_id: Uuid, data: Uint8Array | number[]) =>
  call({ SendInput: { session_id, data: Array.from(data) } });

export const resizeTerminal = (session_id: Uuid, cols: number, rows: number) =>
  call({ ResizeTerminal: { session_id, cols, rows } });

// 切片 2.2 删除（设计 §5）：CopySelection 上移到前端 ——
// xterm.js 自持选区,后端不再发 ClipboardCopy 事件,前端用 navigator.clipboard
// 或 tauri-plugin-clipboard-manager 直接写入。

export interface ThemeInfo {
  current_theme: string;
  current_scheme: string;
  available_themes: string[];
  available_schemes: string[];
}

// ===== 文件传输 =====

export const enqueueUpload = (local: PathBuf, remote: string, session_id: Uuid) =>
  call({ EnqueueUpload: { local, remote, session_id } });

export const enqueueDownload = (remote: string, local: PathBuf, session_id: Uuid) =>
  call({ EnqueueDownload: { remote, local, session_id } });

export const pauseTransfer = (task_id: Uuid) => call({ PauseTransfer: { task_id } });
export const resumeTransfer = (task_id: Uuid) => call({ ResumeTransfer: { task_id } });
export const cancelTransfer = (task_id: Uuid) => call({ CancelTransfer: { task_id } });

export const browseRemoteDir = (session_id: Uuid, path: string) =>
  call<{ entries: unknown[] }>({ BrowseRemoteDir: { session_id, path } });

// ===== 隧道 =====

export const createTunnel = (session_id: Uuid, rule: PortForwardRule) =>
  call({ CreateTunnel: { session_id, rule } });
export const closeTunnel = (tunnel_id: Uuid) => call({ CloseTunnel: { tunnel_id } });
export const listTunnels = () => call<{ tunnels: unknown[] }>({ ListTunnels: null });
export const listPendingTunnels = () => call({ ListPendingTunnels: null });
export const restoreTunnel = (session_id: Uuid, rule: PortForwardRule) =>
  call({ RestoreTunnel: { session_id, rule } });
export const suspendTunnel = (tunnel_id: Uuid) => call({ SuspendTunnel: { tunnel_id } });
export const resumeTunnel = (tunnel_id: Uuid) => call({ ResumeTunnel: { tunnel_id } });

// ===== 快速命令 =====

export const executeQuickCommand = (command_id: Uuid, target_sessions: Uuid[]) =>
  call({ ExecuteQuickCommand: { command_id, target_sessions } });
export const createQuickCommand = (command: QuickCommand) =>
  call({ CreateQuickCommand: { command } });
export const deleteQuickCommand = (command_id: Uuid) =>
  call({ DeleteQuickCommand: { command_id } });
export const listQuickCommands = () => call({ ListQuickCommands: null });

// ===== 触发器 =====

export const createTrigger = (trigger: Trigger) => call({ CreateTrigger: { trigger } });
export const deleteTrigger = (trigger_id: Uuid) => call({ DeleteTrigger: { trigger_id } });
export const toggleTrigger = (trigger_id: Uuid) => call({ ToggleTrigger: { trigger_id } });
export const listTriggers = () => call({ ListTriggers: null });

// ===== 撰写窗格 =====

export const sendComposeText = (content: string, target: ComposeTarget) =>
  call({ SendComposeText: { content, target } });

// ===== 脚本 =====

export const executeScript = (code: string, session_id: Uuid) =>
  call({ ExecuteScript: { code, session_id } });

// ===== 同步输入 =====

export const toggleSyncInput = (session_ids: Uuid[]) =>
  call({ ToggleSyncInput: { session_ids } });

// ===== 密钥管理 =====

export const generateSshKey = (name: string, key_type: SshKeyType, passphrase: string | null) =>
  call({ GenerateSshKey: { name, key_type, passphrase } });
export const importPrivateKey = (path: PathBuf, passphrase: string | null) =>
  call({ ImportPrivateKey: { path, passphrase } });
export const deleteSshKey = (key_id: Uuid) => call({ DeleteSshKey: { key_id } });
export const exportPublicKey = (key_id: Uuid) => call({ ExportPublicKey: { key_id } });
// 切片 3 修正：ListKeys 直接返回数组（切片 2.2 死循环修复后契约对齐）
export const listKeys = () => call<unknown[]>({ ListKeys: null });

// ===== 主密码 =====

export const setupMasterPassword = (password: string) =>
  call({ SetupMasterPassword: { password } });
// 切片 3 修正：VerifyMasterPassword 直接返回 bool（切片 2.2 CommandOutcome::Verified）
export const verifyMasterPassword = (password: string) =>
  call<boolean>({ VerifyMasterPassword: { password } });
export const changeMasterPassword = (old_password: string, new_password: string) =>
  call({ ChangeMasterPassword: { old_password, new_password } });

// ===== 主机密钥 =====

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
export const deleteHostKey = (host: string, port: number) =>
  call({ DeleteHostKey: { host, port } });

// ===== 主题 =====

export const setAppTheme = (theme_name: string) => call({ SetAppTheme: { theme_name } });
export const setTerminalColorScheme = (scheme_name: string) =>
  call({ SetTerminalColorScheme: { scheme_name } });
export const importColorScheme = (scheme: TerminalColorScheme) =>
  call({ ImportColorScheme: { scheme } });
export const listThemes = () =>
  call<{
    current_theme: string;
    current_scheme: string;
    available_themes: string[];
    available_schemes: string[];
  }>({ ListThemes: null });

// 切片 4：decideHostKey —— 已在下方"主机密钥"分区声明（line 196）

// ===== 多协议 =====

export const connectTelnet = (config: TelnetConfig) =>
  call({ ConnectTelnet: { config } });
export const connectSerial = (config: SerialConfig) =>
  call({ ConnectSerial: { config } });

// ===== 插件 =====

export const scanPlugins = () => call({ ScanPlugins: null });
export const loadPlugin = (plugin_id: string) => call({ LoadPlugin: { plugin_id } });
export const unloadPlugin = (plugin_id: string) => call({ UnloadPlugin: { plugin_id } });
export const enablePlugin = (plugin_id: string) => call({ EnablePlugin: { plugin_id } });
export const disablePlugin = (plugin_id: string) => call({ DisablePlugin: { plugin_id } });
export const listPlugins = () => call<{ plugins: unknown[] }>({ ListPlugins: null });

// 重新导出类型方便使用方
export type { ConnectionState, Protocol, ProtocolType };