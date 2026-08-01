/**
 * 切片 0.4 的最小 Vitest + @vue/test-utils 基座示例。
 *
 * 验证：pinia store 的创建/取值/修改三件套在 jsdom 下能跑通。
 * 切片 1 起，每个 slice 新增的 store 都按此模式补单测（设计 §6.2）。
 */
import { describe, it, expect, beforeEach } from "vitest";
import { setActivePinia, createPinia } from "pinia";
import { defineStore } from "pinia";

// 切片 0 示例 store —— 切片 1 起替换为真实 sessions store。
const useCounterStore = defineStore("counter", {
  state: () => ({ value: 0 }),
  actions: {
    inc() {
      this.value += 1;
    },
    reset() {
      this.value = 0;
    },
  },
});

describe("pinia counter store (sample)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("初始值为 0", () => {
    const store = useCounterStore();
    expect(store.value).toBe(0);
  });

  it("inc 后自增", () => {
    const store = useCounterStore();
    store.inc();
    store.inc();
    expect(store.value).toBe(2);
  });

  it("reset 归零", () => {
    const store = useCounterStore();
    store.inc();
    store.reset();
    expect(store.value).toBe(0);
  });
});