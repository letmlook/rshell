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
 * 能力边界(docs/08「核心接口与界面边界」)：密钥管理仅为托管存储——
 * 此处生成/导入的私钥没有「关联到会话」入口、不参与会话认证；会话公钥
 * 认证使用会话自身配置的外部私钥文件路径。界面提示与该边界一致。
 */
import { onMounted, ref } from "vue";
import {
  listKeys,
  generateSshKey,
  importPrivateKey,
  deleteSshKey,
} from "../ipc/client";
import type { SshKeyType, Uuid } from "../ipc/types";
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

interface KeyRow {
  id: Uuid;
  name: string;
  key_type: string;
  fingerprint: string;
  has_passphrase: boolean;
}

const keys = ref<KeyRow[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const generating = ref(false);
const genName = ref("");
const genType = ref<SshKeyType>("ED25519");
const genPassphrase = ref("");

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
    keys.value = (await listKeys()) as unknown as KeyRow[];
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

onMounted(refresh);
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

    <!-- R2-08：能力边界提示，与 docs/08「核心接口与界面边界」保持一致 -->
    <p class="hint">
      密钥管理仅为托管存储：此处生成/导入的私钥不参与会话认证（会话公钥认证使用会话自身配置的外部私钥文件）。
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
      <el-table-column label="操作" width="80">
        <template #default="{ row }">
          <el-button size="small" type="danger" @click="remove(row.id)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>

    <el-dialog
      :model-value="importState !== null"
      title="导入私钥"
      width="440px"
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
</style>
