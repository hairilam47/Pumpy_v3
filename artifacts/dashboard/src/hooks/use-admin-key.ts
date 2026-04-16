import { useState, useEffect, useCallback } from "react";

const HOUR_MS = 60 * 60 * 1000;

// Module-level in-memory admin key cache — no browser storage.
// Shared across all hook instances in the same JS module lifetime.
let _cachedKey = "";
let _cacheExpiry = 0;
let _expiryTimer: ReturnType<typeof setTimeout> | null = null;
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

function scheduleExpiry(ttlMs: number) {
  if (_expiryTimer !== null) clearTimeout(_expiryTimer);
  _expiryTimer = setTimeout(() => {
    _cachedKey = "";
    _cacheExpiry = 0;
    _expiryTimer = null;
    broadcast(); // triggers re-render in all mounted hook instances → adminKey → ""
  }, ttlMs);
}

/**
 * Point-of-use freshness check: reads directly from module state rather than
 * relying on potentially stale React component state.
 * Use this in action onClick handlers before firing privileged mutations.
 */
export function getValidAdminKey(): string {
  return readKey();
}

export interface AdminKeyHook {
  adminKey: string;
  /** Set the key with a 5-minute working TTL — used while typing / for the current action. */
  setAdminKey: (key: string) => void;
  /** Persist the key for 1 hour — call on successful action when "Remember" is checked. */
  rememberAdminKey: (key: string) => void;
  /** Immediately clear the key from memory and cancel any pending expiry timer. */
  clearAdminKey: () => void;
}

/**
 * Admin key hook — pure in-memory cache with automatic TTL expiry (Task #29).
 *
 * - adminKey state tracks current valid key; auto-clears to "" at TTL via timer.
 * - setAdminKey() stores key for 5 min (temporary, covers current action).
 * - rememberAdminKey() stores key for 1 hour (user opted in).
 * - clearAdminKey() wipes the key immediately and cancels the expiry timer.
 * - getValidAdminKey() (exported fn) reads freshly from module state at point of use.
 *
 * No sessionStorage or localStorage — cache is scoped to JS module lifetime,
 * reset on hard page refresh.
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
    const ttl = 5 * 60 * 1000;
    _cachedKey = key;
    _cacheExpiry = key ? Date.now() + ttl : 0;
    if (key) {
      scheduleExpiry(ttl);
    } else {
      if (_expiryTimer !== null) { clearTimeout(_expiryTimer); _expiryTimer = null; }
    }
    broadcast();
  }, []);

  const rememberAdminKey = useCallback((key: string) => {
    _cachedKey = key;
    _cacheExpiry = Date.now() + HOUR_MS;
    scheduleExpiry(HOUR_MS);
    broadcast();
  }, []);

  const clearAdminKey = useCallback(() => {
    _cachedKey = "";
    _cacheExpiry = 0;
    if (_expiryTimer !== null) { clearTimeout(_expiryTimer); _expiryTimer = null; }
    broadcast();
  }, []);

  return { adminKey, setAdminKey, rememberAdminKey, clearAdminKey };
}
