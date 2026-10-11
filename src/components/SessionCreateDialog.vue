<script setup lang="ts">
import { ref, watch } from "vue";
import { useSessionsStore } from "../stores/sessions";
import { listKeys } from "../ipc/client";
import { pickLocalPath } from "../utils/pathPicker";
import type { Protocol, SerialFlowControl, SerialParity, SessionConfig, Uuid } from "../ipc/types";

const props = defineProps<{ visible: boolean }>();
const emit = defineEmits<{ (e: "close"): void; (e: "created", id: Uuid): void }>();

const store = useSessionsStore();

type AuthMode = "password" | "managed-key" | "external-key";

interface KeyRow {
  id: Uuid;
  name: string;
  key_type: string;
  fingerprint: string;
  has_passphrase: boolean;
}

const form = ref({
  name: "",
  protocol: "SSH" as Protocol,
  host: "",
  port: 22,
  username: "",
  password: "",
  authMode: "password" as AuthMode,
  managedKeyId: "" as string,
  keyPassphrase: "",
  externalKeyPath: "",
  serialPort: "",
  baudRate: 115200,
  dataBits: 8,
  stopBits: 1,
  parity: "None" as SerialParity,
  flowControl: "None" as SerialFlowControl,
});

/** 托管密钥列表：对话框打开（SSH 协议）时拉取一次 */
const managedKeys = ref<KeyRow[]>([]);
const managedKeysLoaded = ref(false);
const submitting = ref(false);
const error = ref<string | null>(null);

async function loadManagedKeys() {
  if (managedKeysLoaded.value) return;
  try {
    managedKeys.value = (await listKeys()) as unknown as KeyRow[];
    managedKeysLoaded.value = true;
    if (managedKeys.value.length > 0 && !form.value.managedKeyId) {
      form.value.managedKeyId = managedKeys.value[0].id;
    }
  } catch {
    // 列表拉取失败不阻塞对话框：用户切回密码认证或稍后重试
    managedKeys.value = [];
  }
}

const selectedManagedKey = () =>
  managedKeys.value.find((k) => k.id === form.value.managedKeyId) ?? null;

async function pickExternalKeyPath() {
  const path = await pickLocalPath({ mode: "file", title: "选择私钥文件" });
  if (path) form.value.externalKeyPath = path;
}

watch(() => props.visible, (visible) => {
  if (!visible) {
    form.value.password = "";
    form.value.keyPassphrase = "";
  } else if (form.value.protocol === "SSH") {
    void loadManagedKeys();
  }
}, { immediate: true });

function close() {
  form.value.password = "";
  form.value.keyPassphrase = "";
  emit("close");
}

async function submit() {
  submitting.value = true;
  error.value = null;
  try {
    const isSerial = form.value.protocol === "Serial";
    if (isSerial ? !form.value.serialPort.trim() : !form.value.host.trim()) {
      throw new Error(isSerial ? "请输入串口设备路径" : "请输入主机地址");
    }
    if (form.value.protocol === "SSH" && !form.value.username.trim()) {
      throw new Error("请输入 SSH 用户名");
    }

    let auth_method: SessionConfig["auth_method"];
    let credential: { secret: string } | null = null;
    if (!isSerial && form.value.protocol === "SSH") {
      if (form.value.authMode === "managed-key") {
        if (!form.value.managedKeyId) {
          throw new Error("请选择托管密钥（可在「密钥管理」面板生成或导入）");
        }
        auth_method = {
          PublicKey: {
            username: form.value.username,
            key_path: "",
            has_passphrase: form.value.keyPassphrase.length > 0,
            managed_key_id: form.value.managedKeyId,
          },
        };
        credential = form.value.keyPassphrase.length > 0 ? { secret: form.value.keyPassphrase } : null;
      } else if (form.value.authMode === "external-key") {
        if (!form.value.externalKeyPath.trim()) {
          throw new Error("请选择私钥文件路径");
        }
        auth_method = {
          PublicKey: {
            username: form.value.username,
            key_path: form.value.externalKeyPath,
            has_passphrase: form.value.keyPassphrase.length > 0,
          },
        };
        credential = form.value.keyPassphrase.length > 0 ? { secret: form.value.keyPassphrase } : null;
      } else {
        auth_method = {
          Password: {
            username: form.value.username,
            has_password: form.value.password.length > 0,
          },
        };
        credential = form.value.password.length > 0 ? { secret: form.value.password } : null;
      }
    } else {
      auth_method = {
        Password: {
          username: "",
          has_password: false,
        },
      };
    }

    const cfg: SessionConfig = {
      id: crypto.randomUUID() as Uuid,
      name: form.value.name || (isSerial ? form.value.serialPort : form.value.protocol === "SSH" ? `${form.value.username}@${form.value.host}` : `${form.value.host}:${form.value.port}`),
      folder_id: null,
      host: isSerial ? form.value.serialPort : form.value.host,
      port: isSerial ? 0 : form.value.port,
      protocol: form.value.protocol,
      auth_method,
      serial_config: isSerial ? {
        port: form.value.serialPort,
        baud_rate: form.value.baudRate,
        data_bits: form.value.dataBits,
        stop_bits: form.value.stopBits,
        parity: form.value.parity,
        flow_control: form.value.flowControl,
      } : null,
    };
    const id = await store.create(cfg, credential);
    form.value.password = "";
    form.value.keyPassphrase = "";
    emit("created", id);
    close();
  } catch (e) {
    error.value = String(e);
  } finally {
    submitting.value = false;
  }
}

function onUpdateVisible(v: boolean) {
  if (!v) close();
}
</script>

<template>
  <el-dialog
    :model-value="props.visible"
    title="新建连接"
    width="480px"
    @update:model-value="onUpdateVisible"
    @close="close"
  >
    <el-form label-width="80px" @submit.prevent="submit">
      <el-form-item label="协议">
        <el-select v-model="form.protocol">
          <el-option label="SSH" value="SSH" />
          <el-option label="Telnet" value="Telnet" />
          <el-option label="串口" value="Serial" />
        </el-select>
      </el-form-item>
      <el-form-item label="名称">
        <el-input v-model="form.name" placeholder="可留空,用 host 自动命名" />
      </el-form-item>
      <el-form-item v-if="form.protocol !== 'Serial'" label="主机" required>
        <el-input v-model="form.host" placeholder="host or ip" />
      </el-form-item>
      <el-form-item v-if="form.protocol !== 'Serial'" label="端口">
        <el-input-number v-model="form.port" :min="1" :max="65535" />
      </el-form-item>
      <el-form-item v-if="form.protocol === 'SSH'" label="用户名" required>
        <el-input v-model="form.username" />
      </el-form-item>
      <el-form-item v-if="form.protocol === 'SSH'" label="认证">
        <el-radio-group v-model="form.authMode">
          <el-radio-button value="password">密码</el-radio-button>
          <el-radio-button value="managed-key">托管密钥</el-radio-button>
          <el-radio-button value="external-key">外部私钥</el-radio-button>
        </el-radio-group>
      </el-form-item>
      <el-form-item v-if="form.protocol === 'SSH' && form.authMode === 'password'" label="密码">
        <el-input v-model="form.password" type="password" show-password />
      </el-form-item>
      <template v-if="form.protocol === 'SSH' && form.authMode === 'managed-key'">
        <el-form-item label="密钥">
          <el-select v-model="form.managedKeyId" placeholder="选择托管密钥">
            <el-option
              v-for="key in managedKeys"
              :key="key.id"
              :label="`${key.name} (${key.key_type})`"
              :value="key.id"
            />
          </el-select>
        </el-form-item>
        <el-form-item v-if="selectedManagedKey()?.has_passphrase" label="密钥口令">
          <el-input v-model="form.keyPassphrase" type="password" show-password />
        </el-form-item>
        <p v-if="managedKeys.length === 0" class="auth-hint">
          尚无托管密钥——可在「密钥管理」面板生成或导入后重试。
        </p>
      </template>
      <template v-if="form.protocol === 'SSH' && form.authMode === 'external-key'">
        <el-form-item label="私钥文件" required>
          <el-input v-model="form.externalKeyPath" placeholder="/path/to/id_ed25519" readonly>
            <template #append>
              <el-button @click="pickExternalKeyPath">选择…</el-button>
            </template>
          </el-input>
        </el-form-item>
        <el-form-item label="密钥口令">
          <el-input v-model="form.keyPassphrase" type="password" show-password placeholder="私钥未加密则留空" />
        </el-form-item>
      </template>
      <template v-if="form.protocol === 'Serial'">
        <el-form-item label="设备" required>
          <el-input v-model="form.serialPort" placeholder="/dev/cu.usbserial-..." />
        </el-form-item>
        <el-form-item label="波特率">
          <el-input-number v-model="form.baudRate" :min="1" :max="4000000" />
        </el-form-item>
        <el-form-item label="数据位">
          <el-select v-model="form.dataBits">
            <el-option v-for="bits in [5, 6, 7, 8]" :key="bits" :label="String(bits)" :value="bits" />
          </el-select>
        </el-form-item>
        <el-form-item label="停止位">
          <el-select v-model="form.stopBits">
            <el-option :value="1" label="1" />
            <el-option :value="2" label="2" />
          </el-select>
        </el-form-item>
        <el-form-item label="校验">
          <el-select v-model="form.parity">
            <el-option label="无" value="None" />
            <el-option label="偶" value="Even" />
            <el-option label="奇" value="Odd" />
          </el-select>
        </el-form-item>
        <el-form-item label="流控">
          <el-select v-model="form.flowControl">
            <el-option label="无" value="None" />
            <el-option label="软件" value="Software" />
            <el-option label="硬件" value="Hardware" />
          </el-select>
        </el-form-item>
      </template>
      <p v-if="error" style="color: var(--el-color-danger)">{{ error }}</p>
    </el-form>
    <template #footer>
      <el-button @click="close">取消</el-button>
      <el-button type="primary" :loading="submitting" @click="submit">创建</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.auth-hint {
  margin: 0;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
