import React, { useEffect, useLayoutEffect, useRef, useState } from "react";

const DEFAULT_EXPAND_MS = 175;
const DEFAULT_COLLAPSE_MS = 145;
const DEFAULT_EASE = "cubic-bezier(0.22, 1, 0.36, 1)";
const DEFAULT_SCROLL_SELECTOR =
  "[data-flip-scroll], [data-bxm-scroll], .wmb-tree-scroll, .tree-scroll, .struct-table-container, .right-column";

const activeFlipByContainer = new WeakMap<HTMLElement, () => void>();
let activeFlipFallback: (() => void) | null = null;

function getActiveFlip(container: HTMLElement | null): (() => void) | null {
  return container ? activeFlipByContainer.get(container) ?? null : activeFlipFallback;
}

function setActiveFlip(container: HTMLElement | null, fn: (() => void) | null): void {
  if (container) {
    if (fn) activeFlipByContainer.set(container, fn);
    else activeFlipByContainer.delete(container);
  } else {
    activeFlipFallback = fn;
  }
}

function collectVisibleFollowingSiblings(
  nodeWrapper: HTMLElement | null,
  scrollEl: HTMLElement | null,
  extraBottomMargin: number
): HTMLElement[] {
  const scrollRect = scrollEl?.getBoundingClientRect();
  const maxBottom = scrollRect
    ? scrollRect.bottom + extraBottomMargin + 80
    : window.innerHeight + extraBottomMargin + 80;
  const minTop = scrollRect ? scrollRect.top - 80 : -80;

  const result: HTMLElement[] = [];
  let cur: HTMLElement | null = nodeWrapper;
  while (cur && cur !== scrollEl && result.length < 48) {
    let sib = cur.nextElementSibling as HTMLElement | null;
    while (sib && result.length < 48) {
      const rect = sib.getBoundingClientRect();
      if (rect.height > 0 && rect.bottom >= minTop && rect.top <= maxBottom) {
        result.push(sib);
      }
      sib = sib.nextElementSibling as HTMLElement | null;
    }
    cur = cur.parentElement;
  }
  return result;
}

interface AnimatedBranchProps {
  expanded: boolean;
  children: React.ReactNode;
  className?: string;
  scrollSelector?: string;
  animate?: boolean;
  expandMs?: number;
  collapseMs?: number;
}

export const AnimatedBranch: React.FC<AnimatedBranchProps> = ({
  expanded,
  children,
  className,
  scrollSelector = DEFAULT_SCROLL_SELECTOR,
  animate = true,
  expandMs = DEFAULT_EXPAND_MS,
  collapseMs = DEFAULT_COLLAPSE_MS,
}) => {
  const [hasMounted, setHasMounted] = useState(expanded);
  const outerRef = useRef<HTMLDivElement | null>(null);
  const prevExpandedRef = useRef(expanded);
  const ownFinishRef = useRef<(() => void) | null>(null);
  const ownContainerRef = useRef<HTMLElement | null>(null);

  if (expanded && !hasMounted) {
    setHasMounted(true);
  }

  useLayoutEffect(() => {
    const prevExpanded = prevExpandedRef.current;
    prevExpandedRef.current = expanded;

    if (prevExpanded === expanded) return;

    const outer = outerRef.current;
    if (!outer) return;

    const nodeWrapper = outer.parentElement;
    const scrollEl = nodeWrapper?.closest<HTMLElement>(scrollSelector) ?? null;

    const existingFinish = getActiveFlip(scrollEl);
    if (existingFinish) {
      existingFinish();
    }

    const reducedMotion =
      typeof window !== "undefined" &&
      window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;

    if (!animate || reducedMotion || typeof outer.animate !== "function") {
      outer.style.display = expanded ? "" : "none";
      outer.style.pointerEvents = expanded ? "" : "none";
      return;
    }

    const scrollBottom = scrollEl
      ? scrollEl.getBoundingClientRect().bottom
      : window.innerHeight;

    if (expanded) {
      outer.style.display = "";
      outer.style.pointerEvents = "";

      const outerRect = outer.getBoundingClientRect();
      const fullHeight = outerRect.height;
      if (fullHeight <= 0) return;

      const visibleHeight = Math.min(
        fullHeight,
        Math.max(120, scrollBottom - outerRect.top)
      );
      const offscreenBottom = Math.max(0, fullHeight - visibleHeight);
      const siblings = collectVisibleFollowingSiblings(outer, scrollEl, fullHeight);

      const anims: Animation[] = [];
      const branchAnim = outer.animate(
        [
          { clipPath: `inset(0px 0px ${fullHeight}px 0px)`, opacity: 0 },
          { clipPath: `inset(0px 0px ${offscreenBottom}px 0px)`, opacity: 1 },
        ],
        { duration: expandMs, easing: DEFAULT_EASE }
      );
      anims.push(branchAnim);

      for (const sib of siblings) {
        anims.push(
          sib.animate(
            [
              { transform: `translateY(${-fullHeight}px)` },
              { transform: `translateY(${-offscreenBottom}px)` },
            ],
            { duration: expandMs, easing: DEFAULT_EASE }
          )
        );
      }

      const finish = () => {
        if (getActiveFlip(scrollEl) === finish) setActiveFlip(scrollEl, null);
        if (ownFinishRef.current === finish) ownFinishRef.current = null;
        branchAnim.onfinish = null;
        for (const a of anims) a.cancel();
      };

      ownContainerRef.current = scrollEl;
      ownFinishRef.current = finish;
      setActiveFlip(scrollEl, finish);
      branchAnim.onfinish = finish;
    } else {
      const outerRect = outer.getBoundingClientRect();
      const fullHeight = outerRect.height;
      if (fullHeight <= 0) {
        outer.style.display = "none";
        outer.style.pointerEvents = "none";
        return;
      }

      outer.style.pointerEvents = "none";
      const visibleHeight = Math.min(
        fullHeight,
        Math.max(120, scrollBottom - outerRect.top)
      );
      const offscreenBottom = Math.max(0, fullHeight - visibleHeight);
      const siblings = collectVisibleFollowingSiblings(outer, scrollEl, fullHeight);

      const anims: Animation[] = [];
      const branchAnim = outer.animate(
        [
          { clipPath: `inset(0px 0px ${offscreenBottom}px 0px)`, opacity: 1 },
          { clipPath: `inset(0px 0px ${fullHeight}px 0px)`, opacity: 0 },
        ],
        { duration: collapseMs, easing: DEFAULT_EASE, fill: "forwards" }
      );
      anims.push(branchAnim);

      for (const sib of siblings) {
        anims.push(
          sib.animate(
            [
              { transform: `translateY(${-offscreenBottom}px)` },
              { transform: `translateY(${-fullHeight}px)` },
            ],
            { duration: collapseMs, easing: DEFAULT_EASE, fill: "forwards" }
          )
        );
      }

      const finish = () => {
        if (getActiveFlip(scrollEl) === finish) setActiveFlip(scrollEl, null);
        if (ownFinishRef.current === finish) ownFinishRef.current = null;
        branchAnim.onfinish = null;
        outer.style.display = "none";
        outer.style.pointerEvents = "none";
        for (const a of anims) a.cancel();
      };

      ownContainerRef.current = scrollEl;
      ownFinishRef.current = finish;
      setActiveFlip(scrollEl, finish);
      branchAnim.onfinish = finish;
    }
  }, [expanded, animate, scrollSelector, expandMs, collapseMs]);

  useEffect(() => {
    return () => {
      if (
        ownFinishRef.current &&
        getActiveFlip(ownContainerRef.current) === ownFinishRef.current
      ) {
        ownFinishRef.current();
      }
      ownFinishRef.current = null;
    };
  }, []);

  if (!hasMounted) return null;

  return (
    <div ref={outerRef} className={className}>
      {children}
    </div>
  );
};
