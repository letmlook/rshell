// src/ipc/types.ts - Rust rshell-api 类型的 TypeScript 镜像
//
// 镜像来源:
//   - src-tauri/crates/rshell-api/src/types.rs
//   - src-tauri/crates/rshell-api/src/commands.rs
//   - src-tauri/crates/rshell-api/src/events.rs
//
// 同步原则:
//   - Rust snake_case 字段名在 JSON 序列化时保持 snake_case(serde 默认),
//     所以 TS 端用 snake_case 字段名保持一致。
//   - 数值字段类型(usize/u16/u32/u64/f64)在 TS 统一为 number。
//   - Uuid 序列化为 string(Tauri 自动处理),TS 用 string 表示。
//   - 字节数组 Vec<u8> → number[] 或 Uint8Array(Tauri 通过 Uint8Array 高效传递)。
//   - PathBuf → string(路径字面量)。
//   - Option<T> → T | null。

// ===== 通用 =====
export type Uuid = string;
export type PathBuf = string;
export type Timestamp = string; // ISO 8601 字符串

// ===== 会话相关 =====

export interface SessionConfig {
  id: Uuid;
  name: string;
  folder_id: Uuid | null;
  host: string;
  port: number;
  protocol: Protocol;
  auth_method: AuthMethod;
}

export type Protocol = "SSH" | "Telnet" | "Serial" | "RDP";

export type AuthMethod =
  | { Password: { username: string; password: string } }
  | { PublicKey: { username: string; key_path: PathBuf; passphrase: string | null } }
  | { KeyboardInteractive: { username: string; password: string | null } };

export type ConnectionState =
  | "Connecting"
  | "Connected"
  | "Authenticating"
  | "Disconnecting"
  | "Disconnected";

export interface ConnectionInfo {
  protocol: Protocol;
  host: string;
  port: number;
  state: ConnectionState;
  bytes_sent: number;
  bytes_received: number;
  latency_ms: number | null;
}

export interface SessionInfo {
  id: Uuid;
  config: SessionConfig;
  state: ConnectionState;
}

// ===== 文件传输相关 =====

export interface RemoteFileEntry {
  name: string;
  file_type: FileType;
  size: number;
  permissions: FilePermissions;
  owner: string;
  group: string;
  modified: string;
}

export type FileType = "File" | "Directory" | "Symlink" | "Other";

export interface FilePermissions {
  owner_read: boolean;
  owner_write: boolean;
  owner_execute: boolean;
  group_read: boolean;
  group_write: boolean;
  group_execute: boolean;
  other_read: boolean;
  other_write: boolean;
  other_execute: boolean;
}

export type TransferDirection = "Upload" | "Download";

// ===== 隧道相关 =====

export interface PortForwardRule {
  bind_address: string;
  bind_port: number;
  remote_host: string;
  remote_port: number;
  direction: ForwardDirection;
}

export type ForwardDirection = "Local" | "Remote" | "Dynamic";

export type TunnelState = "Active" | "Suspended" | { Error: string };

// ===== 终端相关 =====

export interface TerminalConfig {
  terminal_type: string;
  encoding: string;
  scrollback_lines: number;
  auto_wrap: boolean;
  cursor_style: CursorStyle;
  cursor_blink: boolean;
}

export type CursorStyle = "Block" | "Underline" | "Bar";

export interface TerminalBufferSnapshot {
  rows: number;
  cols: number;
  cells: CellView[];
  cursor_row: number;
  cursor_col: number;
  cursor_visible: boolean;
  title: string;
}

export interface CellView {
  character: string;
  fg_color: [number, number, number, number]; // RGBA u8 数组
  bg_color: [number, number, number, number];
  flags: number; // u8 位标志
}

export const CellFlag = {
  BOLD: 0b0000_0001,
  DIM: 0b0000_0010,
  ITALIC: 0b0000_0100,
  UNDERLINE: 0b0000_1000,
  BLINK: 0b0001_0000,
  REVERSE: 0b0010_0000,
  HIDDEN: 0b0100_0000,
  STRIKETHROUGH: 0b1000_0000,
} as const;

export interface SearchMatch {
  row: number;
  col: number;
  length: number;
}

// ===== 快速命令相关 =====

export interface QuickCommand {
  id: Uuid;
  name: string;
  command: string;
  send_enter: boolean;
  description: string;
  scope: QuickCommandScope;
  hotkey: string | null;
  group: string | null;
}

export type QuickCommandScope =
  | "CurrentSession"
  | "AllSessions"
  | { SelectedSessions: Uuid[] };

// ===== 触发器相关 =====

export interface Trigger {
  id: Uuid;
  name: string;
  enabled: boolean;
  condition: TriggerCondition;
  action: TriggerAction;
}

export type TriggerCondition =
  | { RegexAppear: string }
  | { ExactMatch: string };

export type TriggerAction =
  | { SendText: string }
  | { ShowNotification: string }
  | "Disconnect"
  | { LogToFile: PathBuf };

// ===== 撰写窗格相关 =====

export type ComposeTarget =
  | "CurrentSession"
  | "AllSessions"
  | { SelectedSessions: Uuid[] };

// ===== 脚本相关 =====

export interface ScriptResult {
  success: boolean;
  output: string;
  error: string | null;
}

// ===== 安全模块相关 =====

export interface SshKeyInfo {
  id: Uuid;
  name: string;
  key_type: SshKeyType;
  fingerprint: string;
  public_key_blob: string;
  comment: string;
  has_passphrase: boolean;
  created_at: Timestamp;
}

export type SshKeyType =
  | "RSA2048"
  | "RSA4096"
  | "ED25519"
  | "ECDSA256"
  | "ECDSA384"
  | "ECDSA521";

export interface ActiveTunnelInfo {
  id: Uuid;
  session_id: Uuid;
  rule: PortForwardRule;
  state: TunnelState;
  bytes_transferred: number;
  connections_count: number;
}

export interface HostKeyEntry {
  host: string;
  port: number;
  key_type: string;
  fingerprint: string;
  trust_level: TrustLevel;
  first_seen: Timestamp;
  last_seen: Timestamp;
}

export type TrustLevel = "Trusted" | "Unknown" | "Mismatch";

export type TrustHostKeyDecision = "TrustOnce" | "TrustPermanent" | "Reject";

// ===== 主题/配色方案 =====

export interface AppTheme {
  name: string;
  mode: ThemeMode;
  colors: ThemeColors;
}

export type ThemeMode = "Light" | "Dark" | "System";

export interface ThemeColors {
  background: number;
  foreground: number;
  accent: number;
  border: number;
  sidebar_bg: number;
  toolbar_bg: number;
  statusbar_bg: number;
  selection_bg: number;
  hover_bg: number;
}

export interface TerminalColorScheme {
  name: string;
  ansi_colors: number[]; // 长度 16
  default_fg: number;
  default_bg: number;
  cursor_fg: number;
  cursor_bg: number;
  selection_fg: number;
  selection_bg: number;
}

// ===== 多协议配置 =====

export type ProtocolType = "SSH" | "Telnet" | "Serial" | "RDP";

export interface SerialConfig {
  port: string;
  baud_rate: number;
  data_bits: number;
  stop_bits: number;
  parity: SerialParity;
  flow_control: SerialFlowControl;
}

export type SerialParity = "None" | "Even" | "Odd";
export type SerialFlowControl = "None" | "Software" | "Hardware";

export interface TelnetConfig {
  host: string;
  port: number;
  terminal_type: string;
}

export interface RdpConfig {
  host: string;
  port: number;
  username: string;
  domain: string | null;
  width: number;
  height: number;
}

// ===== 插件 =====

export interface PluginInfo {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  plugin_type: PluginType;
  state: PluginState;
  extensions: string[];
  permissions: string[];
}

export type PluginType = "Builtin" | "Wasm" | "DynamicLib";
export type PluginState = "Discovered" | "Loaded" | "Active" | "Error" | "Disabled";

// ============================================================================
// AppCommand — 前端 → 后端
// ============================================================================

export type AppCommand =
  // 会话
  | { ConnectSession: { session_id: Uuid } }
  | { DisconnectSession: { session_id: Uuid } }
  | { CreateSession: { config: SessionConfig } }
  | { UpdateSession: { id: Uuid; config: SessionConfig } }
  | { DeleteSession: { id: Uuid } }
  // 终端
  | { SendInput: { session_id: Uuid; data: number[] } }
  | { ResizeTerminal: { session_id: Uuid; cols: number; rows: number } }
  | { CopySelection: { session_id: Uuid } }
  // 传输
  | { EnqueueUpload: { local: PathBuf; remote: string; session_id: Uuid } }
  | { EnqueueDownload: { remote: string; local: PathBuf; session_id: Uuid } }
  | { PauseTransfer: { task_id: Uuid } }
  | { ResumeTransfer: { task_id: Uuid } }
  | { CancelTransfer: { task_id: Uuid } }
  | { BrowseRemoteDir: { session_id: Uuid; path: string } }
  // 隧道
  | { CreateTunnel: { session_id: Uuid; rule: PortForwardRule } }
  | { CloseTunnel: { tunnel_id: Uuid } }
  | { ListPendingTunnels: null }
  | { RestoreTunnel: { session_id: Uuid; rule: PortForwardRule } }
  // 快速命令
  | { ExecuteQuickCommand: { command_id: Uuid; target_sessions: Uuid[] } }
  | { CreateQuickCommand: { command: QuickCommand } }
  | { DeleteQuickCommand: { command_id: Uuid } }
  // 触发器
  | { CreateTrigger: { trigger: Trigger } }
  | { DeleteTrigger: { trigger_id: Uuid } }
  | { ToggleTrigger: { trigger_id: Uuid } }
  // 撰写窗格
  | { SendComposeText: { content: string; target: ComposeTarget } }
  // 脚本
  | { ExecuteScript: { code: string; session_id: Uuid } }
  // 同步输入
  | { ToggleSyncInput: { session_ids: Uuid[] } }
  // 密钥管理
  | { GenerateSshKey: { name: string; key_type: SshKeyType; passphrase: string | null } }
  | { ImportPrivateKey: { path: PathBuf; passphrase: string | null } }
  | { DeleteSshKey: { key_id: Uuid } }
  | { ExportPublicKey: { key_id: Uuid } }
  // 主密码
  | { SetupMasterPassword: { password: string } }
  | { VerifyMasterPassword: { password: string } }
  | { ChangeMasterPassword: { old_password: string; new_password: string } }
  // 主机密钥
  | {
      TrustHostKey: {
        host: string;
        port: number;
        key_type: string;
        public_key_blob: string;
        decision: TrustHostKeyDecision;
      };
    }
  | { DecideHostKey: { decision_id: Uuid; accept: boolean; permanent: boolean } }
  | { DeleteHostKey: { host: string; port: number } }
  // 隧道
  | { SuspendTunnel: { tunnel_id: Uuid } }
  | { ResumeTunnel: { tunnel_id: Uuid } }
  // 主题
  | { SetAppTheme: { theme_name: string } }
  | { SetTerminalColorScheme: { scheme_name: string } }
  | { ImportColorScheme: { scheme: TerminalColorScheme } }
  // 多协议
  | { ConnectTelnet: { config: TelnetConfig } }
  | { ConnectSerial: { config: SerialConfig } }
  // 插件
  | { ScanPlugins: null }
  | { LoadPlugin: { plugin_id: string } }
  | { UnloadPlugin: { plugin_id: string } }
  | { EnablePlugin: { plugin_id: string } }
  | { DisablePlugin: { plugin_id: string } }
  // List / snapshot
  | { ListSessions: null }
  | { ListTunnels: null }
  | { ListKeys: null }
  | { ListPlugins: null }
  | { ListTriggers: null }
  | { ListQuickCommands: null }
  | { ListThemes: null };

// ============================================================================
// AppEvent — 后端 → 前端
// ============================================================================

export type AppEvent =
  // 连接
  | {
      ConnectionStateChanged: {
        session_id: Uuid;
        state: ConnectionState;
        info: ConnectionInfo | null;
      };
    }
  // 终端
  | { TerminalOutput: { session_id: Uuid; data: number[] } }
  | { TerminalTitleChanged: { session_id: Uuid; title: string } }
  | { TerminalBufferUpdated: { session_id: Uuid; snapshot: TerminalBufferSnapshot } }
  // 会话
  | { SessionListChanged: null }
  | { SessionUpdated: { session_id: Uuid } }
  // 传输
  | {
      TransferProgress: {
        task_id: Uuid;
        bytes: number;
        total: number;
        speed_bps: number;
      };
    }
  | { TransferCompleted: { task_id: Uuid } }
  | { TransferFailed: { task_id: Uuid; error: string } }
  | { TransferQueueChanged: null }
  | {
      TransferTaskAdded: {
        task_id: Uuid;
        filename: string;
        direction: TransferDirection;
      };
    }
  | { TransferTaskCompleted: { task_id: Uuid } }
  | { TransferTaskFailed: { task_id: Uuid; error: string } }
  // 远程目录
  | {
      RemoteDirListed: {
        session_id: Uuid;
        path: string;
        entries: RemoteFileEntry[];
      };
    }
  // 隧道
  | { TunnelStateChanged: { tunnel_id: Uuid; state: TunnelState } }
  // 安全
  | {
      HostKeyMismatch: {
        decision_id: Uuid;
        host: string;
        port: number;
        key_type: string;
        expected: string;
        received: string;
        public_key_blob: string;
      };
    }
  | { MasterPasswordRequired: null }
  // 效率工具
  | { QuickCommandListChanged: null }
  | { TriggerListChanged: null }
  | {
      TriggerFired: {
        trigger_id: Uuid;
        session_id: Uuid;
        action_summary: string;
      };
    }
  | { PendingTunnelsSnapshot: { rules: [Uuid, PortForwardRule][] } }
  | { ScriptFinished: { session_id: Uuid; result: ScriptResult } }
  | { SyncInputSessionsChanged: { session_ids: Uuid[] } }
  // 密钥
  | { SshKeyListChanged: null }
  | { SshKeyGenerated: { key: SshKeyInfo } }
  | { PublicKeyExported: { key_id: Uuid; public_key: string } }
  | { MasterPasswordChanged: { is_set: boolean } }
  | { MasterPasswordVerified: { success: boolean } }
  // 隧道
  | { ActiveTunnelsChanged: null }
  | { TunnelUpdated: { tunnel: ActiveTunnelInfo } }
  // 主题
  | { ThemeChanged: { theme: AppTheme } }
  | { ColorSchemeChanged: { scheme: TerminalColorScheme } }
  | { ColorSchemeListChanged: null }
  // 剪贴板
  | { ClipboardCopy: { text: string } }
  // 插件
  | { PluginListUpdated: null }
  | { PluginStateChanged: { plugin_id: string; state: PluginState } }
  | { PluginLoadFailed: { plugin_id: string; error: string } }
  // 快照
  | { SessionsSnapshot: { sessions: SessionConfig[] } }
  | { TunnelsSnapshot: { tunnels: ActiveTunnelInfo[] } }
  | { KeysSnapshot: { keys: SshKeyInfo[] } }
  | { PluginsSnapshot: { plugins: PluginInfo[] } }
  | {
      ThemesSnapshot: {
        current_theme: string;
        current_scheme: string;
        available_themes: string[];
        available_schemes: string[];
      };
    };

// ============================================================================
// 工具:把判别式 union 扁平化成 {variant_name: payload} 用于 invoke 参数
// ============================================================================

/**
 * 把 Rust enum 的 `{ Variant { field: T } }` 形式转成 TS 的 `{ Variant: { field: T } }`。
 * 后端 serde 在跨 IPC 时直接展开变体名作为 key,所以前端用这个 helper 构造 invoke 参数。
 */
export function packCommand(cmd: AppCommand): Record<string, unknown> {
  // TS 的 discriminated union 已经是 {Variant: payload} 形式,直接返回
  return cmd as Record<string, unknown>;
}

/**
 * 把后端发来的事件展开成 discriminated union。
 * Tauri emit 的事件载荷就是 {Variant: payload} 格式,直接 cast 即可。
 */
export function unpackEvent(payload: unknown): AppEvent {
  return payload as AppEvent;
}