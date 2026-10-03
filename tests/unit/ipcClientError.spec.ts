import { describe, expect, it, vi } from "vitest";

// R2-02：后端 IpcError 经 Tauri 2 整体序列化，前端 catch 到的是
// { kind, message, session_id } 对象；client.ts 的 call() 必须把它转成
// 可读 Error，否则所有消费面的 String(e) 都显示 "[object Object]"。
const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import { connectSession, deleteSession, ipcErrorMessage, IpcCallError } from "../../src/ipc/client";

describe("ipc client error shape conversion (R2-02)", () => {
  it("converts a serialized IpcError object into a readable error", async () => {
    invokeMock.mockRejectedValueOnce({
      kind: "not_found",
      message: "Session 404 not found",
      session_id: "session-1",
    });
    const err: unknown = await connectSession("session-1").then(
      () => {
        throw new Error("IpcError 形状的 rejection 必须被拒绝");
      },
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(IpcCallError);
    expect(err).toBeInstanceOf(Error);
    const ipc = err as IpcCallError;
    expect(ipc.message).toBe("Session 404 not found");
    expect(ipc.kind).toBe("not_found");
    expect(ipc.session_id).toBe("session-1");
    // 消费面全是 String(e)：必须显示 message，不得出现 [object Object]
    expect(String(ipc)).toContain("Session 404 not found");
    expect(String(ipc)).not.toContain("[object Object]");
  });

  it("exposes a null session_id when the backend omits it", async () => {
    invokeMock.mockRejectedValueOnce({ kind: "connection", message: "refused", session_id: null });
    const err = (await deleteSession("session-2").catch((e: unknown) => e)) as IpcCallError;
    expect(err).toBeInstanceOf(IpcCallError);
    expect(err.kind).toBe("connection");
    expect(err.session_id).toBeNull();
  });

  it("passes plain string rejections through unchanged", async () => {
    // tauri-plugin-fs 等以字符串 reject；FileBrowserPane.isEntryGone 依赖
    // 原始字符串匹配（enoent / no such file），不得被包装改变行为
    invokeMock.mockRejectedValueOnce("ENOENT: no such file or directory");
    await expect(deleteSession("session-3")).rejects.toBe("ENOENT: no such file or directory");
  });

  it("passes non-IpcError object and Error rejections through unchanged", async () => {
    const boom = new Error("network down");
    invokeMock.mockRejectedValueOnce(boom);
    await expect(deleteSession("session-4")).rejects.toBe(boom);

    invokeMock.mockRejectedValueOnce({ code: -1, data: "opaque" });
    const err = await deleteSession("session-5").catch((e: unknown) => e);
    expect(err).toEqual({ code: -1, data: "opaque" });
    expect(err).not.toBeInstanceOf(IpcCallError);
  });

  it("ipcErrorMessage renders the backend message for terminal Channel paths that bypass call()", () => {
    // TerminalPane.attach_terminal 直接 invoke，绕过 call()；错误状态条文案
    // 必须取 message 而非 "[object Object]"
    expect(
      ipcErrorMessage({ kind: "not_found", message: "Session gone", session_id: null }),
    ).toBe("Session gone");
    expect(ipcErrorMessage(new Error("plain"))).toBe("plain");
    expect(ipcErrorMessage("raw string")).toBe("raw string");
    expect(ipcErrorMessage({ opaque: true })).toBe("[object Object]");
  });
});
