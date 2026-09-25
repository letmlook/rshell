import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import TunnelPanel from "../../src/components/TunnelPanel.vue";
import { listPendingTunnels } from "../../src/ipc/client";

vi.mock("../../src/ipc/client", () => ({
  listTunnels: vi.fn().mockResolvedValue([]),
  listPendingTunnels: vi.fn().mockResolvedValue({ rules: [], unsupported: [] }),
  createTunnel: vi.fn(),
  closeTunnel: vi.fn(),
}));

describe("TunnelPanel", () => {
  const shell = { template: "<div><slot /></div>" };
  const stubs = {
    "el-button": { template: "<button><slot /></button>" },
    "el-option": {
      props: ["label", "value"],
      template: '<option :value="value">{{ label }}</option>',
    },
    "el-select": { template: "<select><slot /></select>" },
    "el-form-item": shell,
    "el-form": shell,
    "el-input": { template: "<input />" },
    "el-empty": { template: "<div />" },
    "el-table-column": shell,
    "el-table": shell,
  };

  beforeEach(() => {
    vi.mocked(listPendingTunnels).mockResolvedValue({ rules: [], unsupported: [] });
  });

  it("offers only forwarding modes backed by SSH", () => {
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    const choices = wrapper.findAll("option").map((option) => option.attributes("value"));
    expect(choices).toEqual(["Local", "Dynamic"]);
  });

  it("shows a skipped legacy Remote rule to the user", async () => {
    vi.mocked(listPendingTunnels).mockResolvedValue({
      rules: [],
      unsupported: [{ session_id: "legacy-session", reason: "Remote forwarding is not supported" }],
    });
    const wrapper = mount(TunnelPanel, { global: { stubs } });
    await flushPromises();
    expect(wrapper.text()).toContain("旧隧道规则已跳过");
    expect(wrapper.text()).toContain("legacy-session");
  });
});
