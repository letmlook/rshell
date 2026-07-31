// TerminalPanel - 终端面板 (xterm.js + 搜索栏)
// 设计规范 §8.2.3.2 / §8.2.3.3
//
// 字节流经 Tauri Channel 推送(后续接入 TerminalOutput 事件 + Channel)。
// 当前阶段先展示静态的"占位输出",确保 xterm 渲染正常。

import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { useAppStore } from "../store/useAppStore";
import { FileManager } from "./FileManager";

interface TerminalPanelProps {
  sessionId: string;
}

export function TerminalPanel({ sessionId }: TerminalPanelProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({
      fontFamily:
        '"JetBrains Mono", "Fira Code", "Cascadia Code", Consolas, monospace',
      fontSize: 13,
      lineHeight: 1.3,
      cursorBlink: true,
      cursorStyle: "block",
      theme: {
        background: "#000000",
        foreground: "#E5E5E5",
        cursor: "#E5E5E5",
        selectionBackground: "rgba(37, 99, 235, 0.3)",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(containerRef.current);

    try {
      fit.fit();
    } catch {
      /* 容器尺寸未稳定时可忽略 */
    }

    termRef.current = term;
    fitRef.current = fit;

    // 占位输出 - 后续接 TerminalOutput 事件
    term.writeln("\x1b[32muser@web-prod-02\x1b[0m:\x1b[34m~\x1b[0m$ ls -la");
    term.writeln("total 32");
    term.writeln("drwxr-xr-x  5 user user 4096 Jul 29 10:20 .");
    term.writeln("drwxr-xr-x  3 root root 4096 Jul 28 14:30 ..");
    term.writeln("-rw-r--r--  1 user user  220 Jul 28 14:30 .bash_logout");
    term.writeln("-rw-r--r--  1 user user 3771 Jul 28 14:30 .bashrc");
    term.writeln("drwxr-xr-x  2 user user 4096 Jul 29 10:20 logs");
    term.writeln("");
    term.write("\x1b[32muser@web-prod-02\x1b[0m:\x1b[34m~\x1b[0m$ ");

    const onResize = () => {
      try {
        fit.fit();
      } catch {
        /* ignore */
      }
    };
    window.addEventListener("resize", onResize);

    return () => {
      window.removeEventListener("resize", onResize);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [sessionId]);

  return (
    <div className="terminal-panel">
      <div
        ref={containerRef}
        className="terminal"
        style={{ width: "100%", height: "100%" }}
      />
      <TerminalSearch />
    </div>
  );
}

function TerminalSearch() {
  return (
    <div className="terminal-search">
      <input type="text" placeholder="搜索..." />
      <button className="search-btn" title="上一个匹配">▲</button>
      <button className="search-btn" title="下一个匹配">▼</button>
      <span className="search-counter">0/0</span>
      <button className="search-btn" title="正则">.*</button>
      <button className="search-btn" title="区分大小写">Aa</button>
      <button className="search-btn" title="关闭搜索">×</button>
    </div>
  );
}

/** 工作区主面板路由:有标签时显示当前标签,没有时显示占位 */
export function Workspace() {
  const tabs = useAppStore((s) => s.tabs);
  const activeTabIndex = useAppStore((s) => s.activeTabIndex);
  const setActiveSessionId = useAppStore((s) => s.setActiveSessionId);

  if (activeTabIndex === null || tabs.length === 0) {
    return (
      <div
        className="workspace"
        style={{ alignItems: "center", justifyContent: "center", color: "var(--text-disabled)" }}
      >
        从左侧选择会话并连接以开始
      </div>
    );
  }

  const tab = tabs[activeTabIndex];
  if (!tab) return null;

  // 把当前激活的 session 同步给 store,后续 Channel 推送用
  setActiveSessionId(tab.sessionId);

  if (tab.type === "sftp") {
    return <FileManager />;
  }

  return <TerminalPanel sessionId={tab.sessionId} />;
}