import { useState, useCallback } from "react";

const SESSION_KEY = "pumpy_admin_key";
const TTL_MS = 60 * 60 * 1000; // 1 hour

interface StoredKey {
  value: string;
  expiresAt: number;
}

function readFromSession(): string {
  try {
    const raw = sessionStorage.getItem(SESSION_KEY);
    if (!raw) return "";
    const parsed = JSON.parse(raw) as StoredKey;
    if (Date.now() > parsed.expiresAt) {
      sessionStorage.removeItem(SESSION_KEY);
      return "";
    }
    return parsed.value;
  } catch {
    return "";
  }
}

function writeToSession(value: string): void {
  try {
    if (!value) {
      sessionStorage.removeItem(SESSION_KEY);
      return;
    }
    const entry: StoredKey = { value, expiresAt: Date.now() + TTL_MS };
    sessionStorage.setItem(SESSION_KEY, JSON.stringify(entry));
  } catch {
    // sessionStorage may be unavailable in some browser configs
  }
}

/**
 * Admin key hook with 1-hour sessionStorage TTL (Task #29).
 * Initialises from session on mount so the user doesn't have to re-enter
 * the key after a soft page refresh within the same browser session.
 */
export function useAdminKey(): [string, (key: string) => void] {
  const [adminKey, setAdminKeyState] = useState<string>(() => readFromSession());

  const setAdminKey = useCallback((value: string) => {
    writeToSession(value);
    setAdminKeyState(value);
  }, []);

  return [adminKey, setAdminKey];
}
