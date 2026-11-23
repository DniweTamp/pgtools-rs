import { useCallback, useSyncExternalStore } from "react";

let currentOverride: boolean | null = null;
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): boolean | null {
  return currentOverride;
}

export function setSharedB2LayoutOverride(next: boolean | null): void {
  if (currentOverride === next) return;
  currentOverride = next;
  listeners.forEach((fn) => fn());
}

export function useB2LayoutOverride(): [
  boolean | null,
  (next: boolean | null) => void,
] {
  const value = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const setValue = useCallback((next: boolean | null) => {
    setSharedB2LayoutOverride(next);
  }, []);
  return [value, setValue];
}
