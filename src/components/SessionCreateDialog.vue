<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useSessionsStore } from "../stores/sessions";
import { listKeys, listSerialPorts } from "../ipc/client";
import { useFoldersStore } from "../stores/folders";
import { pickLocalPath } from "../utils/pathPicker";
import type { Protocol, SerialFlowControl, SerialParity, SessionConfig, Uuid } from "../ipc/types";

const props = defineProps<{ visible: boolean; editId?: Uuid | null }>();
const emit = defineEmits<{
  (e: "close"): void;
  (e: "created", id: Uuid): void;
  (e: "updated", id: Uuid): void;
}>();

const store = useSessionsStore();
const folders = useFoldersStore();

// 键盘交互认证在后端完整实现（AuthMethod::KeyboardInteractive + SSH 客户端的
// 交互式认证流程），此前界面没有这个选项，等于该能力无法使用。
type AuthMode = "password" | "managed-key" | "external-key" | "keyboard-interactive";

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
  /** 所属分组 id；null = 默认分组 */
  folderId: "" as string,
  serialPort: "",
  baudRate: 115200,
  dataBits: 8,
  stopBits: 1,
  parity: "None" as SerialParity,
  flowControl: "None" as SerialFlowControl,
});

/** 可用串口设备：串口协议时拉取一次 */
const serialPorts = ref<string[]>([]);
/** 串口设备的手工输入（列表里没有的设备路径） */
const serialPortsError = ref<string | null>(null);

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

async function loadSerialPorts() {
  serialPortsError.value = null;
  try {
    serialPorts.value = await listSerialPorts();
  } catch (e) {
    // 枚举失败不该挡住手工输入路径
    serialPorts.value = [];
    serialPortsError.value = String(e);
  }
}

const selectedManagedKey = () =>
  managedKeys.value.find((k) => k.id === form.value.managedKeyId) ?? null;

/** 是否处于「编辑已有连接」模式 */
const isEdit = computed(() => !!props.editId);

/**
 * 把已有连接填进表单（编辑模式）。
 *
 * 凭据本体读不出来（存在钥匙串里），所以密码/口令输入框留空：
 * 留空表示「保持现有凭据」，不是「清空凭据」。
 */
function loadForEdit(id: Uuid) {
  const session = store.items.find((item) => item.id === id);
  if (!session) {
    error.value = "连接不存在或已被删除";
    return;
  }
  const auth = session.auth_method as {
    Password?: { username: string; has_password: boolean };
    KeyboardInteractive?: { username: string; has_password: boolean };
    PublicKey?: {
      username: string;
      key_path: string;
      has_passphrase: boolean;
      managed_key_id?: string | null;
    };
  };
  form.value = {
    name: session.name,
    protocol: session.protocol,
    host: session.host,
    port: session.port,
    username: "",
    password: "",
    authMode: "password",
    managedKeyId: "",
    keyPassphrase: "",
    externalKeyPath: "",
    folderId: session.folder_id ?? "",
    serialPort: session.serial_config?.port ?? "",
    baudRate: session.serial_config?.baud_rate ?? 115200,
    dataBits: session.serial_config?.data_bits ?? 8,
    stopBits: session.serial_config?.stop_bits ?? 1,
    parity: session.serial_config?.parity ?? "None",
    flowControl: session.serial_config?.flow_control ?? "None",
  };

  if (auth.PublicKey) {
    form.value.username = auth.PublicKey.username;
    if (auth.PublicKey.managed_key_id) {
      form.value.authMode = "managed-key";
      form.value.managedKeyId = auth.PublicKey.managed_key_id;
    } else {
      form.value.authMode = "external-key";
      form.value.externalKeyPath = auth.PublicKey.key_path;
    }
  } else if (auth.KeyboardInteractive) {
    form.value.authMode = "keyboard-interactive";
    form.value.username = auth.KeyboardInteractive.username;
  } else if (auth.Password) {
    form.value.authMode = "password";
    form.value.username = auth.Password.username;
  }
}

async function pickExternalKeyPath() {
  const path = await pickLocalPath({ mode: "file", title: "选择私钥文件" });
  if (path) form.value.externalKeyPath = path;
}

// 对话框开着时用户把协议改成「串口」，也要拉设备列表
watch(
  () => form.value.protocol,
  (protocol) => {
    if (protocol === "Serial" && props.visible) void loadSerialPorts();
  },
);

watch(
  () => [props.visible, props.editId] as const,
  ([visible, editId]) => {
    if (!visible) {
      form.value.password = "";
      form.value.keyPassphrase = "";
      return;
    }
    error.value = null;
    void folders.refresh();
    if (form.value.protocol === "Serial") void loadSerialPorts();
    if (editId) {
      loadForEdit(editId);
      if (form.value.protocol === "SSH") void loadManagedKeys();
      return;
    }
    if (form.value.protocol === "SSH") void loadManagedKeys();
  },
  { immediate: true },
);

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
      } else if (form.value.authMode === "keyboard-interactive") {
        auth_method = {
          KeyboardInteractive: {
            username: form.value.username,
            has_password: form.value.password.length > 0,
          },
        };
        credential = form.value.password.length > 0 ? { secret: form.value.password } : null;
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
      id: props.editId ?? (crypto.randomUUID() as Uuid),
      name: form.value.name || (isSerial ? form.value.serialPort : form.value.protocol === "SSH" ? `${form.value.username}@${form.value.host}` : `${form.value.host}:${form.value.port}`),
      // 分组 id：留空 = 默认分组
      folder_id: (form.value.folderId || null) as Uuid | null,
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

    if (props.editId) {
      // 编辑：凭据输入框留空 = 保持现有凭据；填了新值才替换。
      // （凭据本体读不出来，留空不能当成「清空」。）
      const credentialUpdate = credential
        ? ({ Set: credential } as const)
        : ("Keep" as const);
      await store.update(props.editId, cfg, credentialUpdate);
      form.value.password = "";
      form.value.keyPassphrase = "";
      emit("updated", props.editId);
      close();
      return;
    }

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
    :title="isEdit ? '编辑连接' : '新建连接'"
    width="480px"
    @update:model-value="onUpdateVisible"
    @close="close"
  >
    <p v-if="isEdit" class="edit-hint" data-test="session-edit-hint">
      密码与密钥口令留空表示<strong>保持现有凭据不变</strong>；填入新值才会替换钥匙串里的那条。
    </p>
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
      <el-form-item label="分组">
        <el-select
          v-model="form.folderId"
          placeholder="默认"
          clearable
          style="width: 100%"
          data-test="session-folder"
        >
          <el-option label="默认" value="" />
          <el-option
            v-for="folder in folders.items"
            :key="folder.id"
            :label="folder.name"
            :value="folder.id"
          />
        </el-select>
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
        <el-radio-group v-model="form.authMode" data-test="session-auth-mode">
          <el-radio-button value="password">密码</el-radio-button>
          <el-radio-button value="managed-key">托管密钥</el-radio-button>
          <el-radio-button value="external-key">外部私钥</el-radio-button>
          <el-radio-button value="keyboard-interactive">键盘交互</el-radio-button>
        </el-radio-group>
      </el-form-item>
      <el-form-item
        v-if="form.protocol === 'SSH' && (form.authMode === 'password' || form.authMode === 'keyboard-interactive')"
        label="密码"
      >
        <el-input v-model="form.password" type="password" show-password />
      </el-form-item>
      <p
        v-if="form.protocol === 'SSH' && form.authMode === 'keyboard-interactive'"
        class="auth-hint"
        data-test="session-keyboard-interactive-hint"
      >
        键盘交互认证：服务器以提示串询问密码时自动应答（适用于要求「密码 + 动态口令」的服务器）。
      </p>
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
          <el-select
            v-if="serialPorts.length"
            v-model="form.serialPort"
            placeholder="选择串口设备"
            filterable
            allow-create
            default-first-option
            style="width: 100%"
            data-test="session-serial-port"
          >
            <el-option
              v-for="port in serialPorts"
              :key="port"
              :label="port"
              :value="port"
            />
          </el-select>
          <el-input
            v-else
            v-model="form.serialPort"
            placeholder="/dev/cu.usbserial-..."
            data-test="session-serial-port"
          />
          <p v-if="serialPortsError" class="auth-hint" data-test="session-serial-ports-error">
            未能枚举串口设备（{{ serialPortsError }}），请手工填写设备路径。
          </p>
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
      <el-button type="primary" :loading="submitting" data-test="session-dialog-submit" @click="submit">
        {{ isEdit ? "保存" : "创建" }}
      </el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.edit-hint {
  margin: 0 0 12px;
  padding: 8px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--el-text-color-secondary);
  background: var(--el-fill-color-light);
  border-radius: 4px;
}
.auth-hint {
  margin: 0;
  font-size: 12px;
  color: var(--el-text-color-secondary);
}
</style>
