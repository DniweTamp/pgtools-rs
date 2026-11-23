import { useEffect, useState } from "react";

export const MENU_CLOSE_DURATION_MS = 115;

export function useClosingTransition<T>(
  value: T | null,
  durationMs: number = MENU_CLOSE_DURATION_MS
): { displayed: T | null; isClosing: boolean } {
  const [prev, setPrev] = useState<T | null>(value);
  const [closing, setClosing] = useState<T | null>(null);

  if (value !== prev) {
    if (value !== null) {
      setClosing(null);
    } else if (prev !== null) {
      setClosing(prev);
    }
    setPrev(value);
  }

  useEffect(() => {
    if (closing === null) return;
    const captured = closing;
    const timer = window.setTimeout(() => {
      setClosing((cur) => (cur === captured ? null : cur));
    }, durationMs);
    return () => window.clearTimeout(timer);
  }, [closing, durationMs]);

  return {
    displayed: value ?? closing,
    isClosing: value === null && closing !== null,
  };
}
