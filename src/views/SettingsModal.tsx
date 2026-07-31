// SettingsModal - 设置对话框 (6 页签)
// 设计规范 §8.2.5.1
// 页签: 基本信息 / 连接 / 终端 / 外观 / 安全 / 高级
// 当前阶段: 占位字段,展示结构。后续接入真实配置。

import { useState } from "react";
import { Modal, ModalTabs, FormField } from "../components/Modal";

interface SettingsModalProps {
  open: boolean;
  onClose: () => void;
}

const TABS = ["基本信息", "连接", "终端", "外观", "安全", "高级"];

export function SettingsModal({ open, onClose }: SettingsModalProps) {
  const [activeTab, setActiveTab] = useState(TABS[0]);

  return (
    <Modal
      open={open}
      title="设置"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn primary">保存</button>
        </>
      }
    >
      <ModalTabs tabs={TABS} active={activeTab} onChange={setActiveTab} />
      <div className="modal-body">
        {activeTab === "基本信息" && (
          <>
            <FormField label="会话名称" value="web-prod-02" />
            <FormField label="主机地址" value="192.168.1.101" />
            <FormField label="端口" value="22" />
            <FormField label="协议" value="SSH" readOnly />
            <FormField label="用户名" value="user" />
          </>
        )}
        {activeTab === "连接" && (
          <>
            <FormField label="认证方式" value="密码认证" readOnly />
            <FormField label="连接超时 (秒)" value="30" />
            <FormField label="保活间隔 (秒)" value="60" />
            <FormField label="代理" value="无" />
          </>
        )}
        {activeTab === "终端" && (
          <>
            <FormField label="终端类型" value="xterm-256color" />
            <FormField label="编码" value="UTF-8" />
            <FormField label="回滚行数" value="10000" />
            <FormField label="字体" value="JetBrains Mono" />
            <FormField label="字号" value="13" />
          </>
        )}
        {activeTab === "外观" && (
          <>
            <FormField label="主题" value="Dark" />
            <FormField label="配色方案" value="Default Dark" />
            <FormField label="字体" value="Inter" />
          </>
        )}
        {activeTab === "安全" && (
          <>
            <FormField label="主机密钥策略" value="自动接受" />
            <FormField label="主密码" type="password" />
          </>
        )}
        {activeTab === "高级" && (
          <>
            <FormField label="日志级别" value="info" />
            <FormField label="调试端口" value="0" />
          </>
        )}
      </div>
    </Modal>
  );
}