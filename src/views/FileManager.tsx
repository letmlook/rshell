// FileManager - SFTP 文件管理器(双窗格 + 表头 + 路径栏 + 传输队列嵌入)
// 设计规范 §8.2.4

import { useState } from "react";
import { TransferQueue } from "./TransferQueue";

export function FileManager() {
  return (
    <div className="file-manager">
      <FilePanes />
      <TransferQueue />
    </div>
  );
}

function FilePanes() {
  return (
    <div className="file-panes">
      <FilePane side="local" />
      <FilePane side="remote" />
    </div>
  );
}

interface FilePaneProps {
  side: "local" | "remote";
}

function FilePane({ side }: FilePaneProps) {
  const [path, setPath] = useState(side === "local" ? "/" : "/home");

  // 占位文件列表(后续接 RemoteDirListed 事件)
  const items = [
    { name: "..", isDir: true, size: "-", date: "" },
    { name: "src", isDir: true, size: "-", date: "Jul 28" },
    { name: "tests", isDir: true, size: "-", date: "Jul 27" },
    { name: ".env", isDir: false, size: "1.2KB", date: "Jul 29" },
    { name: "README.md", isDir: false, size: "2.3KB", date: "Jul 25" },
    { name: "package.json", isDir: false, size: "800B", date: "Jul 20" },
  ];

  return (
    <div className="file-pane">
      <div className="file-pane-header">
        <span className="pane-title">{side === "local" ? "本地文件" : "远程文件"}</span>
        <input
          className="pane-path"
          type="text"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          spellCheck={false}
        />
        <button className="search-btn" title="书签">
          🔖
        </button>
      </div>
      <div className="file-list-header">
        <span className="col-name">名称</span>
        <span className="col-size">大小</span>
        <span className="col-date">修改时间</span>
      </div>
      <div className="file-list">
        {items.map((item, idx) => (
          <div
            key={`${item.name}-${idx}`}
            className={`file-item ${idx === 3 ? "selected" : ""}`}
          >
            <span className="file-icon">{item.isDir ? "📁" : "📄"}</span>
            <span className="file-name">{item.name}</span>
            <span className="file-size">{item.size}</span>
            <span className="file-date">{item.date}</span>
          </div>
        ))}
      </div>
    </div>
  );
}