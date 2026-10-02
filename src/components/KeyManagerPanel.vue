<script setup lang="ts">
/**
 * KeyManagerPanel —— 切片 6.3
 *
 * SSH 密钥管理界面 —— 设计 §4.2 "私钥 / 主密码"行:
 * "只出 SshKeyInfo, 私钥永不过 IPC"。
 *
 * 后端 list_keys 返回的元数据(id/name/fingerprint/public_key_blob/...)
 * 展示在这里;用户点击"导入"通过 tauri-plugin-dialog 选本地文件;
 * 真正的解密/SSH 握手在后端 infra::crypto 完成。
 */
import { onMounted, ref } from "vue";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import {
  listKeys,
  generateSshKey,
  importPrivateKey,
  deleteSshKey,
} from "../ipc/client";
import type { SshKeyType, Uuid } from "../ipc/types";

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
  const path = await open({
    multiple: false,
    filters: [{ name: "SSH key", extensions: ["", "pem", "key", "pub"] }],
  });
  if (!path) return;
  importPassphrase.value = "";
  importState.value = { path: path as string };
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
  // wry/WKWebView 不实现 window.confirm, 必须走 plugin-dialog 原生确认框
  const ok = await confirm(`删除 SSH 密钥“${name}”？此操作无法撤销。`, {
    title: "删除密钥",
    kind: "warning",
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
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>
