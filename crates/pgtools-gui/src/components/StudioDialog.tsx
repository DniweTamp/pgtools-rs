import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

interface ConfirmOptions {
  title?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
}

interface PromptOptions {
  title?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  placeholder?: string;
}

type DialogRequest =
  | {
      kind: "alert";
      title: string;
      message: string;
      confirmLabel: string;
      resolve: () => void;
    }
  | {
      kind: "confirm";
      title: string;
      message: string;
      confirmLabel: string;
      cancelLabel: string;
      danger: boolean;
      resolve: (value: boolean) => void;
    }
  | {
      kind: "prompt";
      title: string;
      message: string;
      defaultValue: string;
      placeholder: string;
      confirmLabel: string;
      cancelLabel: string;
      resolve: (value: string | null) => void;
    };

type Listener = (req: DialogRequest | null) => void;

const queue: DialogRequest[] = [];
let activeRequest: DialogRequest | null = null;
const listeners = new Set<Listener>();

function notify() {
  for (const fn of listeners) {
    fn(activeRequest);
  }
}

function enqueue(req: DialogRequest) {
  if (!activeRequest) {
    activeRequest = req;
    notify();
  } else {
    queue.push(req);
  }
}

function advanceQueue() {
  activeRequest = queue.shift() ?? null;
  notify();
}

export function studioAlert(message: string, title = "Notice"): Promise<void> {
  return new Promise((resolve) => {
    enqueue({
      kind: "alert",
      title,
      message,
      confirmLabel: "OK",
      resolve,
    });
  });
}

export function studioConfirm(
  message: string,
  options?: ConfirmOptions
): Promise<boolean> {
  const looksDestructive =
    options?.danger ?? /\b(delete|discard|remove)\b/i.test(message);
  return new Promise((resolve) => {
    enqueue({
      kind: "confirm",
      title: options?.title ?? (looksDestructive ? "Confirm Action" : "Confirm"),
      message,
      confirmLabel: options?.confirmLabel ?? (looksDestructive ? "Delete" : "OK"),
      cancelLabel: options?.cancelLabel ?? "Cancel",
      danger: looksDestructive,
      resolve,
    });
  });
}

export function studioPrompt(
  message: string,
  defaultValue = "",
  options?: PromptOptions
): Promise<string | null> {
  return new Promise((resolve) => {
    enqueue({
      kind: "prompt",
      title: options?.title ?? "Input",
      message,
      defaultValue,
      placeholder: options?.placeholder ?? "",
      confirmLabel: options?.confirmLabel ?? "OK",
      cancelLabel: options?.cancelLabel ?? "Cancel",
      resolve,
    });
  });
}

export const StudioDialogHost: React.FC = () => {
  const [req, setReq] = useState<DialogRequest | null>(activeRequest);
  const [inputVal, setInputVal] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);
  const confirmBtnRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    const listener: Listener = (next) => {
      setReq(next);
      if (next?.kind === "prompt") {
        setInputVal(next.defaultValue);
      }
    };
    listeners.add(listener);
    if (activeRequest?.kind === "prompt") {
      setInputVal(activeRequest.defaultValue);
    }
    return () => {
      listeners.delete(listener);
    };
  }, []);

  useEffect(() => {
    if (!req) return;
    const raf = window.requestAnimationFrame(() => {
      if (req.kind === "prompt" && inputRef.current) {
        inputRef.current.focus();
        inputRef.current.select();
      } else if (confirmBtnRef.current) {
        confirmBtnRef.current.focus();
      }
    });
    return () => window.cancelAnimationFrame(raf);
  }, [req]);

  if (!req) return null;

  const handleCancel = () => {
    const current = req;
    advanceQueue();
    if (current.kind === "alert") {
      current.resolve();
    } else if (current.kind === "confirm") {
      current.resolve(false);
    } else {
      current.resolve(null);
    }
  };

  const handleConfirm = () => {
    const current = req;
    const val = inputVal;
    advanceQueue();
    if (current.kind === "alert") {
      current.resolve();
    } else if (current.kind === "confirm") {
      current.resolve(true);
    } else {
      current.resolve(val);
    }
  };

  return createPortal(
    <div
      className="modal-backdrop studio-dialog-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) {
          handleCancel();
        }
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          handleCancel();
        } else if (e.key === "Enter" && req.kind !== "prompt") {
          e.preventDefault();
          e.stopPropagation();
          handleConfirm();
        }
      }}
    >
      <div
        className="modal-card studio-dialog-card"
        role="dialog"
        aria-modal="true"
        aria-label={req.title}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="studio-dialog-title">{req.title}</div>
        <div className="studio-dialog-message">{req.message}</div>

        {req.kind === "prompt" && (
          <input
            ref={inputRef}
            type="text"
            className="field-box mono studio-dialog-input"
            value={inputVal}
            placeholder={req.placeholder}
            onChange={(e) => setInputVal(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                e.stopPropagation();
                handleConfirm();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                handleCancel();
              }
            }}
          />
        )}

        <div className="studio-dialog-actions">
          {req.kind !== "alert" && (
            <button
              type="button"
              className="pill-button"
              onClick={handleCancel}
            >
              {req.cancelLabel}
            </button>
          )}
          <button
            ref={confirmBtnRef}
            type="button"
            className={
              req.kind === "confirm" && req.danger
                ? "pill-button studio-dialog-danger"
                : "pill-button accent"
            }
            onClick={handleConfirm}
          >
            {req.confirmLabel}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
};
