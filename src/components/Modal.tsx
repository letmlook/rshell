// Modal - 通用模态对话框
// 设计规范 §8.2.5
// 700px 宽 + 48px header + fadeIn/slideUp 动画 + Esc 关闭 + 遮罩点击关闭

import { useEffect, type ReactNode } from "react";

interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
}

export function Modal({ open, title, onClose, children, footer }: ModalProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  return (
    <div
      className={`modal-overlay ${open ? "active" : ""}`}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <span>{title}</span>
          <button className="modal-close" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>
        {children}
        {footer && <div className="modal-footer">{footer}</div>}
      </div>
    </div>
  );
}

interface ModalTabsProps {
  tabs: string[];
  active: string;
  onChange: (tab: string) => void;
}

export function ModalTabs({ tabs, active, onChange }: ModalTabsProps) {
  return (
    <div className="modal-tabs">
      {tabs.map((t) => (
        <button
          key={t}
          className={`modal-tab ${active === t ? "active" : ""}`}
          onClick={() => onChange(t)}
        >
          {t}
        </button>
      ))}
    </div>
  );
}

interface FormFieldProps {
  label: string;
  value?: string;
  placeholder?: string;
  type?: string;
  readOnly?: boolean;
  onChange?: (value: string) => void;
}

export function FormField({
  label,
  value,
  placeholder,
  type = "text",
  readOnly,
  onChange,
}: FormFieldProps) {
  return (
    <div className="form-group">
      <label className="form-label">{label}</label>
      <input
        className="form-input"
        type={type}
        value={value ?? ""}
        placeholder={placeholder}
        readOnly={readOnly}
        onChange={(e) => onChange?.(e.target.value)}
      />
    </div>
  );
}