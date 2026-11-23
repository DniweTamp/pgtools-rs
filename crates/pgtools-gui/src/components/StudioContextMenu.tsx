import React from "react";
import { ContextMenuState } from "../types";

interface StudioContextMenuProps {
  menu: ContextMenuState | null;
  isClosing: boolean;
  displayedSubmenuIdx: number | null;
  isSubmenuClosing: boolean;
  onHoverSubmenu: (idx: number) => void;
  onClose: () => void;
}

export const StudioContextMenu: React.FC<StudioContextMenuProps> = ({
  menu,
  isClosing,
  displayedSubmenuIdx,
  isSubmenuClosing,
  onHoverSubmenu,
  onClose,
}) => {
  if (!menu) return null;
  return (
    <div
      className="context-menu-backdrop"
      style={isClosing ? { pointerEvents: "none" } : undefined}
      onClick={onClose}
      onContextMenu={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <div
        key={`${menu.x}-${menu.y}`}
        className={`context-menu ${isClosing ? "closing" : ""}`}
        style={{ left: menu.x, top: menu.y }}
        onClick={(e) => e.stopPropagation()}
      >
        {menu.items.map((item, idx) =>
          item.separator ? (
            <div key={idx} className="context-menu-sep" />
          ) : (
            <div
              key={idx}
              className="context-menu-item"
              onMouseEnter={() => onHoverSubmenu(idx)}
              onClick={() => {
                if (!item.disabled && item.action) {
                  item.action();
                }
              }}
            >
              <span>{item.label}</span>
              {item.shortcut && (
                <span className="context-menu-shortcut">{item.shortcut}</span>
              )}
              {item.submenu && <span className="context-menu-shortcut">›</span>}
              {item.submenu && displayedSubmenuIdx === idx && (
                <div className={`context-submenu ${isSubmenuClosing ? "closing" : ""}`}>
                  {item.submenu.map((sub, sIdx) =>
                    sub.separator ? (
                      <div key={sIdx} className="context-menu-sep" />
                    ) : (
                      <button
                        key={sIdx}
                        className="context-menu-item"
                        disabled={sub.disabled}
                        onClick={() => {
                          if (sub.action) sub.action();
                        }}
                      >
                        <span>{sub.label}</span>
                      </button>
                    )
                  )}
                </div>
              )}
            </div>
          )
        )}
      </div>
    </div>
  );
};
