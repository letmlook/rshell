// Tree-shakeable Element Plus registration.
//
// Importing from `element-plus` directly resolves to its full es/index.mjs
// entry, which pulls every component into the bundle. We import each
// component from its sub-path so Rollup tree-shakes the unused ones out
// and the production JS chunks stay under the 500 KiB build-output
// threshold enforced by scripts/check-build-output.mjs.
//
// Keep src/styles/element-plus.css in sync with the component list here;
// each import is paired with a per-component theme-chalk CSS file.

import type { App, Plugin } from "vue";

import ElButton, { ElButtonGroup } from "element-plus/es/components/button/index.mjs";
import ElCheckbox from "element-plus/es/components/checkbox/index.mjs";
import ElDialog from "element-plus/es/components/dialog/index.mjs";
import ElDropdown, {
  ElDropdownItem,
  ElDropdownMenu,
} from "element-plus/es/components/dropdown/index.mjs";
import ElEmpty from "element-plus/es/components/empty/index.mjs";
import ElForm, { ElFormItem } from "element-plus/es/components/form/index.mjs";
import ElInput from "element-plus/es/components/input/index.mjs";
import ElInputNumber from "element-plus/es/components/input-number/index.mjs";
import ElLoading from "element-plus/es/components/loading/index.mjs";
import ElSelect, { ElOption } from "element-plus/es/components/select/index.mjs";
import ElSwitch from "element-plus/es/components/switch/index.mjs";
import ElTable, { ElTableColumn } from "element-plus/es/components/table/index.mjs";
import ElTag from "element-plus/es/components/tag/index.mjs";
import ElTooltip from "element-plus/es/components/tooltip/index.mjs";

const componentPlugins: Plugin[] = [
  ElButton,
  ElButtonGroup,
  ElCheckbox,
  ElDialog,
  ElDropdown,
  ElDropdownItem,
  ElDropdownMenu,
  ElEmpty,
  ElForm,
  ElFormItem,
  ElInput,
  ElInputNumber,
  ElLoading,
  ElOption,
  ElSelect,
  ElSwitch,
  ElTable,
  ElTableColumn,
  ElTag,
  ElTooltip,
];

export function registerElementPlus(app: App) {
  for (const plugin of componentPlugins) {
    app.use(plugin);
  }
}

export { ElMessage } from "element-plus/es/components/message/index.mjs";
export { ElMessageBox } from "element-plus/es/components/message-box/index.mjs";
export { ElNotification } from "element-plus/es/components/notification/index.mjs";