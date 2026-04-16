import { useState, useEffect, useCallback } from "react";

const HOUR_MS = 60 * 60 * 1000;

// Module-level in-memory admin key cache — no browser storage.
// Shared across all hook instances in the same JS module lifetime.
let _cachedKey = "";
let _cacheExpiry = 0;
const _listeners = new Set<() => void>();

function readKey(): string {
  if (_cachedKey && Date.now() < _cacheExpiry) return _cachedKey;
  _cachedKey = "";
  _cacheExpiry = 0;
  return "";
}

function broadcast() {
  _listeners.forEach((fn) => fn());
}

export interface AdminKeyHook {
  adminKey: string;
  /** Set the key with a short working TTL (5 min) — used while typing / for the current action. */
  setAdminKey: (key: string) => void;
  /** Persist the key for 1 hour — call on successful action when "Remember" is checked. */
  rememberAdminKey: (key: string) => void;
  /** Immediately clear the key from memory. */
  clearAdminKey: () => void;
}

/**
 * Admin key hook — pure in-memory cache with TTL (Task #29).
 *
 * - readKey()        returns the cached key if within TTL, else ""
 * - setAdminKey()    stores the key for 5 minutes (temporary, covers current action)
 * - rememberAdminKey() stores the key for 1 hour (user opted in)
 * - clearAdminKey()  wipes the key immediately
 *
 * No sessionStorage or localStorage is used — cache is scoped to the
 * JS module lifetime and is reset on hard page refresh.
 */
export function useAdminKey(): AdminKeyHook {
  const [adminKey, setLocalState] = useState(readKey);

  useEffect(() => {
    const sync = () => setLocalState(readKey());
    _listeners.add(sync);
    return () => {
      _listeners.delete(sync);
    };
  }, []);

  const setAdminKey = useCallback((key: string) => {
    _cachedKey = key;
    _cacheExpiry = key ? Date.now() + 5 * 60 * 1000 : 0;
    broadcast();
  }, []);

  const rememberAdminKey = useCallback((key: string) => {
    _cachedKey = key;
    _cacheExpiry = Date.now() + HOUR_MS;
    broadcast();
  }, []);

  const clearAdminKey = useCallback(() => {
    _cachedKey = "";
    _cacheExpiry = 0;
    broadcast();
  }, []);

  return { adminKey, setAdminKey, rememberAdminKey, clearAdminKey };
}
