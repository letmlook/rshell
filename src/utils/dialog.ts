/**
 * 应用内弹窗服务 —— 全局替换系统/原生弹窗的唯一入口。
 *
 * 之前确认与输入类弹窗走 `ElMessageBox`，删除类走 tauri-plugin-dialog 的
 * `confirm()`，选文件走它的 `open()`：前者是 Element Plus 的系统风格窗体，
 * 后者是操作系统原生窗口，风格与本应用完全不一致。用户要求「全局自定义
 * 弹窗，不弹系统窗口」，因此统一收口到这里：
 *
 *   confirmDialog({...})  → Promise<boolean>       替代 ElMessageBox.confirm
 *   promptDialog({...})   → Promise<string | null> 替代 ElMessageBox.prompt
 *   openDialog({...})     → Promise<string | null> 多按钮/自定义窗体
 *
 * 渲染由 `components/dialogs/DialogHost.vue` 负责（挂在 App.vue 里一次）。
 * 与 transferLog 同样的取舍：调用点分散在多个组件，走 props 只能层层透传。
 *
 * 用队列而不是单个槽位：解决一个弹窗的回调里常会立刻再弹一个（冲突确认 →
 * 重命名输入），单槽会把前一个的 Promise 永远悬着。
 */
import { ref, type Ref } from "vue";

export type DialogVariant = "primary" | "danger" | "ghost";

/** 取消语义的按钮值；关闭（Esc/遮罩）与它等价 */
export const DIALOG_CANCEL = "cancel";

export interface DialogButton {
  label: string;
  /** 点击后 resolve 的值 */
  value: string;
  variant?: DialogVariant;
  /** 输入框的确认按钮：点击时才跑校验并回填输入内容 */
  submit?: boolean;
}

export interface DialogInputSpec {
  placeholder?: string;
  value: string;
  /** 返回错误文案表示不通过；返回 null（或不校验）表示通过 */
  validate?: (value: string) => string | null;
}

export interface DialogSpec {
  title: string;
  message?: string;
  /** 补充信息（路径、错误原文），等宽显示 */
  detail?: string;
  buttons: DialogButton[];
  input?: DialogInputSpec;
  /** 点遮罩/Esc 是否关闭，默认 true */
  dismissible?: boolean;
  width?: string;
}

export interface ActiveDialog extends DialogSpec {
  inputError: string | null;
}

interface PendingDialog {
  spec: DialogSpec;
  resolve: (value: string | null) => void;
}

const active = ref<ActiveDialog | null>(null);
/** 当前弹窗；宿主与测试都直接读它（与 transferLog 的 entries 同一套导出方式） */
export { active };
const queue: PendingDialog[] = [];
let activeResolve: ((value: string | null) => void) | null = null;

function showNext() {
  if (active.value || queue.length === 0) return;
  const next = queue.shift()!;
  active.value = { ...next.spec, inputError: null };
  activeResolve = next.resolve;
}

/** 打开一个弹窗；返回被点击按钮的 value，关闭（Esc/遮罩/取消）时返回 null */
export function openDialog(spec: DialogSpec): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    queue.push({ spec, resolve });
    showNext();
  });
}

/** 关闭当前弹窗并结算它的 Promise，随后立刻推下一条 */
export function closeDialog(value: string | null = null) {
  const resolve = activeResolve;
  activeResolve = null;
  active.value = null;
  resolve?.(value);
  showNext();
}

/** 确认框：是 → true，取消/关闭 → false（不抛异常，调用点不必写 try/catch） */
export function confirmDialog(options: {
  title: string;
  message: string;
  detail?: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
}): Promise<boolean> {
  return openDialog({
    title: options.title,
    message: options.message,
    detail: options.detail,
    buttons: [
      { label: options.cancelText ?? "取消", value: DIALOG_CANCEL, variant: "ghost" },
      {
        label: options.confirmText ?? "确定",
        value: "ok",
        variant: options.danger ? "danger" : "primary",
      },
    ],
  }).then((value) => value === "ok");
}

/**
 * 输入框：确认 → 输入的字符串，取消/关闭 → null。
 * 校验失败时窗口保持打开并就地显示原因，不静默关掉让用户重新输入一遍。
 */
export function promptDialog(options: {
  title: string;
  message?: string;
  detail?: string;
  placeholder?: string;
  defaultValue?: string;
  confirmText?: string;
  cancelText?: string;
  validate?: (value: string) => string | null;
}): Promise<string | null> {
  return openDialog({
    title: options.title,
    message: options.message,
    detail: options.detail,
    buttons: [
      { label: options.cancelText ?? "取消", value: DIALOG_CANCEL, variant: "ghost" },
      { label: options.confirmText ?? "确定", value: "ok", variant: "primary", submit: true },
    ],
    input: {
      placeholder: options.placeholder,
      value: options.defaultValue ?? "",
      validate: options.validate,
    },
  }).then((value) => (value === null || value === DIALOG_CANCEL ? null : value));
}

/**
 * 确认按钮的点击处理：有输入框时先校验，通过则把输入内容作为结果回填；
 * 校验不通过只就地显示原因并返回 false（窗口保持打开）。
 */
export function submitActiveDialog(): boolean {
  const current = active.value;
  if (!current) return false;
  if (current.input) {
    const error = current.input.validate?.(current.input.value) ?? null;
    if (error) {
      current.inputError = error;
      return false;
    }
    closeDialog(current.input.value);
    return true;
  }
  const target = current.buttons.find((button) => button.submit)
    ?? current.buttons.find((button) => button.variant === "primary");
  closeDialog(target?.value ?? "ok");
  return true;
}

/** 供 DialogHost 双向绑定输入框内容 */
export function setActiveDialogInput(value: string) {
  if (!active.value?.input) return;
  active.value.input.value = value;
  if (active.value.inputError) active.value.inputError = null;
}

export function useDialog(): {
  active: Ref<ActiveDialog | null>;
  open: typeof openDialog;
  close: typeof closeDialog;
  confirm: typeof confirmDialog;
  prompt: typeof promptDialog;
  submit: typeof submitActiveDialog;
  setInput: typeof setActiveDialogInput;
} {
  return {
    active,
    open: openDialog,
    close: closeDialog,
    confirm: confirmDialog,
    prompt: promptDialog,
    submit: submitActiveDialog,
    setInput: setActiveDialogInput,
  };
}

/** 仅测试用：清空队列与当前弹窗，避免用例之间互相污染 */
export function resetDialogs(): void {
  queue.length = 0;
  active.value = null;
  activeResolve = null;
}
