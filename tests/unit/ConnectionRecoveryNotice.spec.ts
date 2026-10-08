import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import ElementPlus from "element-plus";
import ConnectionRecoveryNotice from "../../src/components/ConnectionRecoveryNotice.vue";
import type { CoreErrorKind } from "../../src/ipc/types";

const cases: Array<{
  kind: CoreErrorKind;
  text: string;
  action: string;
  emitted: "update-credential" | "retry-connect" | "retry-load";
}> = [
  { kind: "credential_missing", text: "凭据条目缺失", action: "更新凭据", emitted: "update-credential" },
  { kind: "credential_inaccessible", text: "修复钥匙串访问后重试", action: "重试连接", emitted: "retry-connect" },
  { kind: "credential_save_failed", text: "凭据保存失败", action: "更新凭据", emitted: "update-credential" },
  { kind: "credential_migration_failed", text: "旧配置迁移失败", action: "重试加载", emitted: "retry-load" },
  { kind: "auth_failed", text: "认证被拒绝", action: "重新编辑凭据", emitted: "update-credential" },
  { kind: "connection", text: "网络连接失败", action: "重试连接", emitted: "retry-connect" },
  { kind: "host_key_mismatch", text: "主机密钥未确认", action: "重新连接", emitted: "retry-connect" },
  { kind: "host_key_trust_persistence_failed", text: "永久信任保存失败", action: "重试连接", emitted: "retry-connect" },
];

describe("CoreError recovery affordances", () => {
  it.each(cases)("$kind renders honest guidance and $action", async ({ kind, text, action, emitted }) => {
    const wrapper = mount(ConnectionRecoveryNotice, {
      props: {
        sessionId: "session-1",
        kind,
        message: "underlying production reason",
      },
      global: { plugins: [ElementPlus] },
    });

    expect(wrapper.text()).toContain(text);
    expect(wrapper.text()).toContain("underlying production reason");
    const button = wrapper.get(`[data-test="recovery-${emitted}"]`);
    expect(button.text()).toContain(action);
    await button.trigger("click");
    expect(wrapper.emitted(emitted)?.[0]).toEqual(["session-1"]);
  });
});
