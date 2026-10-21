<script setup lang="ts">
/**
 * MasterPasswordPanel —— 应用锁定
 *
 * 主密码的语义边界（见 docs/08）：**只用于应用锁定与验证**，不是凭据保险库，
 * 也不加密 macOS 钥匙串里的会话凭据。面板必须如实说明这一点，不能让用户
 * 误以为设了主密码就能保护钥匙串里的密码。
 *
 * 后端只持久化盐与加密后的验证令牌，派生密钥永不落盘：重启后状态是「已锁定」，
 * 需要重新输入主密码解锁。
 */
import { computed, onMounted, ref } from "vue";
import { useMasterPasswordStore } from "../stores/masterPassword";
import { confirmDialog } from "../utils/dialog";

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false });

const store = useMasterPasswordStore();

const newPassword = ref("");
const confirmPassword = ref("");
const oldPassword = ref("");
const changePassword = ref("");
const disablePassword = ref("");
const notice = ref<string | null>(null);

const stateLabel = computed(() => {
  switch (store.state) {
    case "NotSet":
      return "未设置主密码";
    case "Locked":
      return "已设置 · 当前已锁定";
    case "Unlocked":
      return "已设置 · 当前已解锁";
  }
});

function resetForms() {
  newPassword.value = "";
  confirmPassword.value = "";
  oldPassword.value = "";
  changePassword.value = "";
  disablePassword.value = "";
}

async function onSetup() {
  notice.value = null;
  if (newPassword.value.length < 8) {
    notice.value = "主密码至少 8 个字符";
    return;
  }
  if (newPassword.value !== confirmPassword.value) {
    notice.value = "两次输入的主密码不一致";
    return;
  }
  const ok = await store.setup(newPassword.value);
  if (ok) {
    notice.value = "主密码已设置并保存到本机";
    resetForms();
  }
}

async function onChange() {
  notice.value = null;
  if (changePassword.value.length < 8) {
    notice.value = "新主密码至少 8 个字符";
    return;
  }
  const ok = await store.change(oldPassword.value, changePassword.value);
  if (ok) {
    notice.value = "主密码已修改";
    oldPassword.value = "";
    changePassword.value = "";
  }
}

async function onDisable() {
  notice.value = null;
  const confirmed = await confirmDialog({
    title: "停用主密码",
    message: "停用后应用不再要求主密码解锁，本机的 master_password.json 会被删除。",
    confirmText: "停用",
    danger: true,
  });
  if (!confirmed) return;
  const ok = await store.disable(disablePassword.value);
  if (ok) {
    notice.value = "主密码已停用";
    disablePassword.value = "";
  }
}

onMounted(async () => {
  await store.refresh();
});
</script>

<template>
  <section class="master-password-panel">
    <h3 v-if="!props.embedded">主密码</h3>

    <p class="state" data-test="master-password-state">{{ stateLabel }}</p>
    <p v-if="store.error" class="error" data-test="master-password-error">{{ store.error }}</p>
    <p v-if="notice" class="notice" data-test="master-password-notice">{{ notice }}</p>

    <p class="hint">
      主密码只用于<strong>锁定本应用</strong>：重启后需要重新输入主密码才能解锁。
      它不是凭据保险库，也不会加密 macOS 钥匙串里的会话密码或私钥口令。
    </p>

    <template v-if="store.state === 'NotSet'">
      <el-form size="small" label-position="top" @submit.prevent="onSetup">
        <el-form-item label="设置主密码">
          <el-input
            v-model="newPassword"
            type="password"
            show-password
            autocomplete="new-password"
            placeholder="至少 8 个字符"
            data-test="master-password-new"
          />
        </el-form-item>
        <el-form-item label="确认主密码">
          <el-input
            v-model="confirmPassword"
            type="password"
            show-password
            autocomplete="new-password"
            data-test="master-password-confirm"
          />
        </el-form-item>
        <el-button
          type="primary"
          native-type="submit"
          :loading="store.loading"
          data-test="master-password-setup"
        >
          设置主密码
        </el-button>
      </el-form>
    </template>

    <template v-else>
      <el-button
        :disabled="store.loading"
        data-test="master-password-lock"
        @click="store.lock()"
      >
        立即锁定应用
      </el-button>

      <el-form size="small" label-position="top" class="block-form" @submit.prevent="onChange">
        <h4>修改主密码</h4>
        <el-form-item label="当前主密码">
          <el-input
            v-model="oldPassword"
            type="password"
            show-password
            autocomplete="current-password"
            data-test="master-password-old"
          />
        </el-form-item>
        <el-form-item label="新主密码">
          <el-input
            v-model="changePassword"
            type="password"
            show-password
            autocomplete="new-password"
            data-test="master-password-changed"
          />
        </el-form-item>
        <el-button native-type="submit" :loading="store.loading" data-test="master-password-change">
          修改主密码
        </el-button>
      </el-form>

      <el-form size="small" label-position="top" class="block-form" @submit.prevent="onDisable">
        <h4>停用主密码</h4>
        <el-form-item label="当前主密码">
          <el-input
            v-model="disablePassword"
            type="password"
            show-password
            autocomplete="current-password"
            data-test="master-password-disable-input"
          />
        </el-form-item>
        <el-button
          type="danger"
          plain
          native-type="submit"
          :loading="store.loading"
          data-test="master-password-disable"
        >
          停用主密码
        </el-button>
      </el-form>
    </template>
  </section>
</template>

<style scoped>
.master-password-panel {
  padding: 12px;
}
.master-password-panel h3 {
  margin: 0 0 12px;
  font-size: 14px;
}
.master-password-panel h4 {
  margin: 0 0 8px;
  font-size: 12px;
  color: var(--rs-fg-muted);
}
.state {
  margin: 0 0 8px;
  font-size: 12px;
  color: var(--rs-fg);
}
.hint {
  margin: 0 0 12px;
  font-size: 11px;
  line-height: 1.6;
  color: var(--rs-fg-muted);
}
.block-form {
  margin-top: 16px;
  padding-top: 12px;
  border-top: 1px solid var(--rs-border);
}
.error {
  color: var(--el-color-danger);
  font-size: 12px;
}
.notice {
  color: var(--el-color-success);
  font-size: 12px;
}
</style>
