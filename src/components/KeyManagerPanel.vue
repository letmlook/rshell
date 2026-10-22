<script setup lang="ts">
/**
 * KeyManagerPanel —— 切片 6.3
 *
 * SSH 密钥管理界面 —— 设计 §4.2 "私钥 / 主密码"行:
 * "只出 SshKeyInfo, 私钥永不过 IPC"。
 *
 * 后端 list_keys 返回的元数据(id/name/fingerprint/public_key_blob/...)
 * 展示在这里;用户点击"导入"通过应用内路径选择器选本地文件。
 *
 * 能力边界(docs/08「核心接口与界面边界」)：托管私钥可通过「新建连接 →
 * 认证 → 托管密钥」参与会话认证——连接时后端经 KeyManager 取私钥字节，
 * 口令沿用会话自己的钥匙串槽位。私钥本体仍永不过 IPC（只出 SshKeyInfo）。
 */
import { onBeforeUnmount, onMounted, ref } from "vue";
import {
  listKeys,
  generateSshKey,
  importPrivateKey,
  deleteSshKey,
  exportPublicKey,
} from "../ipc/client";
import { subscribeAppEvents } from "../ipc/events";
import type { SshKeyInfo, SshKeyType, Uuid } from "../ipc/types";
import { confirmDialog } from "../utils/dialog";
import { pickLocalPath } from "../utils/pathPicker";

/**
 * 只把「像私钥」的条目列进选择器，减少选到明显无关文件后的失败。
 * 这里是启发式过滤，不替代后端导入时的真实解析校验——选错仍会得到后端错误。
 */
function isLikelyKeyFile(name: string): boolean {
  return /\.(pem|key|ppk|pub|openssh)$/i.test(name) || !/\./.test(name);
}

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

// 密钥行直接用 IPC 契约里的 `SshKeyInfo`（src/ipc/types.ts）：
// 后端 `ListKeys` 返回的就是它，本地再声明一份 `KeyRow` 只能靠 `as unknown as`
// 强转 —— Rust 侧改字段名时不会编译报错，只会在运行时拿到 undefined。
const keys = ref<SshKeyInfo[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const generating = ref(false);
const genName = ref("");
const genType = ref<SshKeyType>("ED25519");
const genPassphrase = ref("");
/** 导出的公钥（后端通过 PublicKeyExported 事件回传，不是命令返回值） */
const exported = ref<{ name: string; publicKey: string } | null>(null);
const exportError = ref<string | null>(null);

let unlisten: (() => void) | null = null;
/**
 * 组件是否仍然存活。
 *
 * `onMounted` 是 async 的（内含 refresh / subscribeAppEvents 两次 await）。
 * 用户在往返窗口内切走面板 → 组件卸载 → `onBeforeUnmount` 已经跑过 →
 * 后续 resolve 的 `subscribeAppEvents` 把 unlisten 赋给一个已销毁的实例，
 * `stop()` 永不执行 → Tauri 事件通道上留下常驻监听器，回调持有已销毁组件的
 * ref；每次切面板泄漏一个。
 *
 * 闩锁模式与 `QuickCommandPanel` / `TriggerEditor` 一致：await 之后检查
 * `active`，不活就立刻 `stop()`。
 */
let active = false;
/** 点击导出时的密钥名：PublicKeyExported 事件只带 key_id 与公钥 */
let pendingExportName = "";

// 生成类型下拉:与后端 SshKeyType(src-tauri/crates/rshell-api/src/types.rs:354)逐一对齐,
// value 由 SshKeyType 类型约束 —— 出现后端不存在的变体(如 "RSA"/"ECDSA")会直接编译报错,
// 避免 serde unknown variant 导致生成必然失败。
const genTypeOptions: ReadonlyArray<{ value: SshKeyType; label: string }> = [
  { value: "ED25519", label: "ED25519" },
  { value: "RSA2048", label: "RSA 2048" },
  { value: "RSA4096", label: "RSA 4096" },
  { value: "ECDSA256", label: "ECDSA 256" },
  { value: "ECDSA384", label: "ECDSA 384" },
  { value: "ECDSA521", label: "ECDSA 521" },
];

// 导入私钥的口令对话框状态: wry/WKWebView 不实现 window.prompt, 需要真实 el-dialog
const importState = ref<{ path: string } | null>(null);
const importPassphrase = ref("");
const importing = ref(false);

async function refresh() {
  loading.value = true;
  error.value = null;
  try {
    keys.value = await listKeys();
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function importKey() {
  // 应用内路径选择器替代 tauri-plugin-dialog 的 open()：不弹操作系统窗口
  const path = await pickLocalPath({
    mode: "file",
    title: "选择私钥文件",
    accept: isLikelyKeyFile,
  });
  if (!path) return;
  importPassphrase.value = "";
  importState.value = { path };
}

async function confirmImport() {
  if (!importState.value || importing.value) return;
  importing.value = true;
  try {
    await importPrivateKey(importState.value.path, importPassphrase.value || null);
    cancelImport();
    await refresh();
  } catch (e) {
    error.value = String(e);
  } finally {
    importing.value = false;
  }
}

function cancelImport() {
  importState.value = null;
  importPassphrase.value = "";
}

async function generate() {
  if (!genName.value) return;
  generating.value = true;
  try {
    await generateSshKey(
      genName.value,
      genType.value,
      genPassphrase.value || null,
    );
    genName.value = "";
    genPassphrase.value = "";
    await refresh();
  } catch (e) {
    error.value = String(e);
  } finally {
    generating.value = false;
  }
}

async function remove(id: Uuid) {
  const name = keys.value.find(key => key.id === id)?.name ?? id;
  // 应用内确认窗替代 plugin-dialog 的 confirm()：不弹操作系统窗口
  const ok = await confirmDialog({
    title: "删除 SSH 密钥",
    message: `删除密钥「${name}」？此操作无法撤销。`,
    confirmText: "删除",
    danger: true,
  });
  if (!ok) return;
  try {
    await deleteSshKey(id);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
}

/**
 * 导出公钥。
 *
 * 后端 `ExportPublicKey` 命令本身返回 None，公钥通过 `PublicKeyExported`
 * 事件回传（命令 → 事件 → 界面），这里等事件到达再开窗体。
 * 私钥本体永不过 IPC，导出的只有公钥。
 */
async function exportKey(id: Uuid) {
  exportError.value = null;
  const name = keys.value.find((key) => key.id === id)?.name ?? id;
  try {
    await exportPublicKey(id);
    // 命令只返回 None，公钥随 PublicKeyExported 事件回来；
    // 事件里没有名称，这里先记下点击时的名称，事件到达后用上。
    pendingExportName = name;
  } catch (e) {
    exportError.value = String(e);
  }
}

/** 把公钥写入系统剪贴板 */
async function copyExported() {
  if (!exported.value) return;
  try {
    await navigator.clipboard.writeText(exported.value.publicKey);
  } catch (e) {
    exportError.value = `复制失败：${String(e)}`;
  }
}

onMounted(async () => {
  active = true;
  await refresh();
  if (!active) return;
  const stop = await subscribeAppEvents((event) => {
    if (typeof event === "string") return;
    if ("PublicKeyExported" in event) {
      const { key_id, public_key } = event.PublicKeyExported;
      exported.value = {
        name:
          keys.value.find((key) => key.id === key_id)?.name ??
          pendingExportName ??
          key_id,
        publicKey: public_key,
      };
      exportError.value = null;
    }
  });
  // 组件可能已在 await 期间被卸载：这时立刻退订，否则监听器会永久留下。
  if (active) unlisten = stop;
  else stop();
});

onBeforeUnmount(() => {
  active = false;
  unlisten?.();
  unlisten = null;
});
</script>

<template>
  <section class="key-manager">
    <header>
      <h3 v-if="!props.embedded">SSH 密钥 ({{ keys.length }})</h3>
      <el-button-group size="small">
        <el-button @click="refresh" :loading="loading">刷新</el-button>
        <el-button @click="importKey">导入</el-button>
      </el-button-group>
    </header>

    <p v-if="error" class="error">{{ error }}</p>

    <!-- 托管密钥已在「新建连接 → 认证 → 托管密钥」接入会话认证；
         删除被引用的密钥会让对应会话连接失败并提示重新选择 -->
    <p class="hint">
      此处生成/导入的私钥可在新建连接时选择用于 SSH 认证（口令在连接时经系统钥匙串提供）。删除仍被连接引用的密钥会导致该连接无法再认证。
    </p>

    <el-form inline size="small" class="gen-form" @submit.prevent="generate">
      <el-form-item label="生成">
        <el-input v-model="genName" placeholder="name" style="width: 120px" />
      </el-form-item>
      <el-form-item>
        <el-select v-model="genType" style="width: 120px">
          <el-option
            v-for="opt in genTypeOptions"
            :key="opt.value"
            :label="opt.label"
            :value="opt.value"
          />
        </el-select>
      </el-form-item>
      <el-form-item>
        <el-input v-model="genPassphrase" type="password" placeholder="passphrase" style="width: 120px" />
      </el-form-item>
      <el-form-item>
        <el-button type="primary" native-type="submit" :loading="generating">生成</el-button>
      </el-form-item>
    </el-form>

    <el-table :data="keys" stripe size="small" empty-text="暂无密钥">
      <el-table-column prop="name" label="名称" />
      <el-table-column prop="key_type" label="类型" width="90" />
      <el-table-column prop="fingerprint" label="指纹" />
      <el-table-column label="口令" width="60">
        <template #default="{ row }">
          {{ row.has_passphrase ? "✓" : "—" }}
        </template>
      </el-table-column>
      <el-table-column label="操作" width="140">
        <template #default="{ row }">
          <el-button
            size="small"
            :data-test="`key-export-${row.id}`"
            @click="exportKey(row.id)"
          >
            导出公钥
          </el-button>
          <el-button size="small" type="danger" @click="remove(row.id)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>

    <p v-if="exportError" class="error" data-test="key-export-error">{{ exportError }}</p>

    <el-dialog
      :model-value="exported !== null"
      title="导出公钥"
      width="560px"
      append-to-body
      @close="exported = null"
    >
      <p class="export-name">{{ exported?.name }}</p>
      <pre class="export-body">{{ exported?.publicKey }}</pre>
      <template #footer>
        <el-button @click="exported = null">关闭</el-button>
        <el-button type="primary" data-test="key-export-copy" @click="copyExported">复制</el-button>
      </template>
    </el-dialog>

    <el-dialog
      :model-value="importState !== null"
      title="导入私钥"
      width="440px"
      append-to-body
      @close="cancelImport"
    >
      <p class="import-path">{{ importState?.path }}</p>
      <el-form label-width="80px" @submit.prevent="confirmImport">
        <el-form-item label="口令">
          <el-input
            v-model="importPassphrase"
            type="password"
            show-password
            autocomplete="new-password"
            placeholder="留空 = 无口令"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="cancelImport">取消</el-button>
        <el-button type="primary" :loading="importing" :disabled="importing" @click="confirmImport">导入</el-button>
      </template>
    </el-dialog>
  </section>
</template>

<style scoped>
.key-manager {
  padding: 12px;
}
header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
h3 {
  margin: 0;
  font-size: 14px;
}
.gen-form {
  margin-bottom: 12px;
}
.import-path {
  margin: 0 0 12px;
  font-family: monospace;
  font-size: 12px;
  color: var(--el-text-color-secondary);
  word-break: break-all;
}
.hint {
  margin: 0 0 12px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
.export-name {
  margin: 0 0 8px;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
.export-body {
  margin: 0;
  padding: 8px;
  max-height: 220px;
  overflow: auto;
  font-family: monospace;
  font-size: 11px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-all;
  background: var(--rs-bg-surface, #1b1b20);
  border: 1px solid var(--rs-border, #2c2c33);
  border-radius: 6px;
}
</style>
