<script setup lang="ts">
/**
 * DialogHost —— 全局弹窗宿主
 *
 * 在 App.vue 里挂一次，渲染 `utils/dialog.ts` 的当前弹窗。目的是让所有确认/
 * 输入窗体都是本应用自己的样式与交互（遮罩、圆角、主题色、Esc、焦点），
 * 不再出现 Element Plus 的系统风格 MessageBox，也不出现操作系统原生窗口。
 *
 * 交互约定：
 *   - Esc / 点遮罩 = 取消（dismissible=false 的窗体除外，如强确认）
 *   - Enter 在输入框里提交；Tab 在窗体内循环（焦点不外泄到背景）
 *   - 打开时焦点落在输入框（有输入）或主按钮上
 */
import { nextTick, ref, watch } from "vue";
import { closeDialog, DIALOG_CANCEL, useDialog } from "../../utils/dialog";

const { active, submit, setInput } = useDialog();

const inputRef = ref<HTMLInputElement | null>(null);
const panelRef = ref<HTMLElement | null>(null);

const FOCUSABLE = "button:not(:disabled), input:not(:disabled), [tabindex]";

/** Esc 关闭 / Enter 提交 / Tab 焦点循环 */
function onKeydown(event: KeyboardEvent) {
  if (!active.value) return;
  if (event.key === "Escape" && (active.value.dismissible ?? true)) {
    event.preventDefault();
    closeDialog(null);
    return;
  }
  if (event.key === "Enter" && active.value.input) {
    // 按钮上的 Enter 交给浏览器默认 click，避免一次按键提交两次
    const target = event.target as HTMLElement | null;
    if (target?.tagName !== "BUTTON") {
      event.preventDefault();
      submit();
    }
    return;
  }
  if (event.key === "Tab" && panelRef.value) {
    const nodes = Array.from(panelRef.value.querySelectorAll<HTMLElement>(FOCUSABLE));
    if (nodes.length === 0) return;
    const first = nodes[0];
    const last = nodes[nodes.length - 1];
    const current = document.activeElement as HTMLElement | null;
    if (!event.shiftKey && current === last) {
      event.preventDefault();
      first.focus();
    } else if (event.shiftKey && (current === first || !current)) {
      event.preventDefault();
      last.focus();
    }
  }
}

function onButtonClick(value: string, submitFlag?: boolean) {
  // 输入框的确认按钮要先过校验：非法时窗口保持打开
  if (submitFlag) {
    if (!submit()) return;
    return;
  }
  closeDialog(value === DIALOG_CANCEL ? null : value);
}

function onBackdropClick() {
  if (active.value?.dismissible === false) return;
  closeDialog(null);
}

// 每次开新窗体都重新对焦：否则焦点还留在上一个弹窗（多半已被卸载）的位置
watch(active, async (dialog) => {
  if (!dialog) return;
  await nextTick();
  if (dialog.input) {
    inputRef.value?.focus();
    inputRef.value?.select();
  } else {
    panelRef.value?.querySelector<HTMLElement>(".dlg-btn.is-primary")?.focus();
  }
});
</script>

<template>
  <Teleport to="body">
    <div
      v-if="active"
      class="dlg-backdrop"
      data-test="dlg-backdrop"
      @mousedown.self="onBackdropClick"
      @keydown="onKeydown"
    >
      <div
        ref="panelRef"
        class="dlg-panel"
        role="dialog"
        aria-modal="true"
        :aria-label="active.title"
        :style="active.width ? { width: active.width } : undefined"
        data-test="dlg-panel"
      >
        <h3 class="dlg-title">{{ active.title }}</h3>
        <p v-if="active.message" class="dlg-message">{{ active.message }}</p>
        <pre v-if="active.detail" class="dlg-detail">{{ active.detail }}</pre>

        <input
          v-if="active.input"
          ref="inputRef"
          class="dlg-input"
          data-test="dlg-input"
          :value="active.input.value"
          :placeholder="active.input.placeholder"
          :aria-invalid="!!active.inputError"
          @input="setInput(($event.target as HTMLInputElement).value)"
        />
        <p v-if="active.inputError" class="dlg-input-error" role="alert" data-test="dlg-input-error">
          {{ active.inputError }}
        </p>

        <div class="dlg-actions">
          <button
            v-for="button in active.buttons"
            :key="button.value"
            type="button"
            class="dlg-btn"
            :class="`is-${button.variant ?? 'ghost'}`"
            :data-test="`dlg-btn-${button.value}`"
            @click="onButtonClick(button.value, button.submit)"
          >
            {{ button.label }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.dlg-backdrop {
  position: fixed;
  inset: 0;
  z-index: 4000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--rs-s-4);
  background: rgb(0 0 0 / 45%);
}
.dlg-panel {
  width: 420px;
  max-width: 100%;
  max-height: 100%;
  display: flex;
  flex-direction: column;
  gap: var(--rs-s-2);
  padding: var(--rs-s-4);
  background: var(--rs-bg-surface);
  color: var(--rs-fg);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-2, 8px);
  box-shadow: 0 12px 32px rgb(0 0 0 / 45%);
  font-family: var(--rs-font-ui);
}
.dlg-title {
  margin: 0;
  font-size: var(--rs-fs-lg, 15px);
  font-weight: 600;
  color: var(--rs-fg);
}
.dlg-message {
  margin: 0;
  font-size: var(--rs-fs-sm, 13px);
  line-height: 1.6;
  color: var(--rs-fg);
  white-space: pre-wrap;
  word-break: break-word;
}
.dlg-detail {
  margin: 0;
  max-height: 160px;
  overflow: auto;
  padding: 6px 8px;
  font-family: var(--rs-font-mono);
  font-size: var(--rs-fs-xs);
  color: var(--rs-fg-muted);
  background: var(--rs-bg-panel);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
  white-space: pre-wrap;
  word-break: break-all;
}
.dlg-input {
  height: 30px;
  padding: 0 8px;
  font-size: var(--rs-fs-sm, 13px);
  font-family: var(--rs-font-mono);
  color: var(--rs-fg);
  background: var(--rs-bg-panel);
  border: 1px solid var(--rs-border);
  border-radius: var(--rs-radius-1);
}
.dlg-input:focus {
  outline: none;
  border-color: var(--rs-accent);
}
.dlg-input[aria-invalid="true"] { border-color: var(--rs-p-danger); }
.dlg-input-error {
  margin: 0;
  font-size: var(--rs-fs-xs);
  color: var(--rs-p-danger);
}
.dlg-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--rs-s-2);
  margin-top: var(--rs-s-2);
}
.dlg-btn {
  min-width: 76px;
  height: 28px;
  padding: 0 var(--rs-s-3);
  font-size: var(--rs-fs-sm, 13px);
  font-family: var(--rs-font-ui);
  border-radius: var(--rs-radius-1);
  cursor: pointer;
  border: 1px solid var(--rs-border);
  background: var(--rs-bg-panel);
  color: var(--rs-fg);
}
.dlg-btn:hover { background: var(--rs-bg-surface-hover); }
.dlg-btn:focus-visible { outline: 1px solid var(--rs-accent); }
.dlg-btn.is-primary {
  background: var(--rs-accent);
  border-color: var(--rs-accent);
  color: #fff;
}
.dlg-btn.is-primary:hover { filter: brightness(1.08); }
.dlg-btn.is-danger {
  background: var(--rs-p-danger);
  border-color: var(--rs-p-danger);
  color: #fff;
}
.dlg-btn.is-danger:hover { filter: brightness(1.08); }
</style>
