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
import { open } from "@tauri-apps/plugin-dialog";
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
  const passphrase = window.prompt("passphrase (留空 = 无口令):") ?? null;
  try {
    await importPrivateKey(path as string, passphrase);
    await refresh();
  } catch (e) {
    error.value = String(e);
  }
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
    <header v-if="!props.embedded">
      <h3>SSH 密钥 ({{ keys.length }})</h3>
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
        <el-select v-model="genType" style="width: 100px">
          <el-option label="ED25519" value="ED25519" />
          <el-option label="RSA" value="RSA" />
          <el-option label="ECDSA" value="ECDSA" />
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
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
</style>