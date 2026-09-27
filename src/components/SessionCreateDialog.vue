<script setup lang="ts">
import { ref } from "vue";
import { useSessionsStore } from "../stores/sessions";
import type { Protocol, SerialFlowControl, SerialParity, SessionConfig, Uuid } from "../ipc/types";

const props = defineProps<{ visible: boolean }>();
const emit = defineEmits<{ (e: "close"): void; (e: "created", id: Uuid): void }>();

const store = useSessionsStore();

const form = ref({
  name: "",
  protocol: "SSH" as Protocol,
  host: "",
  port: 22,
  username: "",
  password: "",
  serialPort: "",
  baudRate: 115200,
  dataBits: 8,
  stopBits: 1,
  parity: "None" as SerialParity,
  flowControl: "None" as SerialFlowControl,
});

const submitting = ref(false);
const error = ref<string | null>(null);

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
    const cfg: SessionConfig = {
      id: crypto.randomUUID() as Uuid,
      name: form.value.name || (isSerial ? form.value.serialPort : form.value.protocol === "SSH" ? `${form.value.username}@${form.value.host}` : `${form.value.host}:${form.value.port}`),
      folder_id: null,
      host: isSerial ? form.value.serialPort : form.value.host,
      port: isSerial ? 0 : form.value.port,
      protocol: form.value.protocol,
      auth_method: {
        Password: {
          username: form.value.protocol === "SSH" ? form.value.username : "",
          has_password: form.value.protocol === "SSH" && form.value.password.length > 0,
        },
      },
      serial_config: isSerial ? {
        port: form.value.serialPort,
        baud_rate: form.value.baudRate,
        data_bits: form.value.dataBits,
        stop_bits: form.value.stopBits,
        parity: form.value.parity,
        flow_control: form.value.flowControl,
      } : null,
    };
    const credential = form.value.protocol === "SSH" && form.value.password.length > 0
      ? { secret: form.value.password }
      : null;
    const id = await store.create(cfg, credential);
    emit("created", id);
    emit("close");
  } catch (e) {
    error.value = String(e);
  } finally {
    submitting.value = false;
  }
}

function onUpdateVisible(v: boolean) {
  if (!v) emit("close");
}
</script>

<template>
  <el-dialog
    :model-value="props.visible"
    title="新建会话"
    width="480px"
    @update:model-value="onUpdateVisible"
    @close="emit('close')"
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
      <el-form-item v-if="form.protocol === 'SSH'" label="密码">
        <el-input v-model="form.password" type="password" show-password />
      </el-form-item>
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
      <el-button @click="emit('close')">取消</el-button>
      <el-button type="primary" :loading="submitting" @click="submit">创建</el-button>
    </template>
  </el-dialog>
</template>
