import { useEffect, useRef, useState } from "react";
import { WifiOff, RefreshCw } from "lucide-react";
import { cn } from "@/lib/utils";

const WS_URL = (() => {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/api/bot/stream`;
})();

const RECONNECT_DELAY_MS = 5_000;
const PROBE_INTERVAL_MS = 15_000;

type ConnectionState = "connecting" | "online" | "offline";

/**
 * OfflineBanner (Task #29)
 *
 * Opens a lightweight WebSocket probe to /api/bot/stream.
 * Shows a dismissible amber banner when the connection is lost.
 * Auto-reconnects every 5 s. Hides when reconnected.
 */
export default function OfflineBanner() {
  const [connState, setConnState] = useState<ConnectionState>("connecting");
  const [dismissed, setDismissed] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  function clearTimer() {
    if (timerRef.current != null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }

  function connect() {
    if (wsRef.current) {
      try { wsRef.current.close(); } catch { /* ignore */ }
    }

    const ws = new WebSocket(WS_URL);
    wsRef.current = ws;

    ws.onopen = () => {
      setConnState("online");
      setDismissed(false);
      // Schedule periodic re-check
      timerRef.current = setTimeout(() => {
        connect();
      }, PROBE_INTERVAL_MS);
    };

    ws.onerror = () => {
      setConnState("offline");
    };

    ws.onclose = () => {
      if (connState !== "online") {
        setConnState("offline");
      } else {
        setConnState("offline");
      }
      timerRef.current = setTimeout(connect, RECONNECT_DELAY_MS);
    };
  }

  useEffect(() => {
    connect();
    return () => {
      clearTimer();
      try { wsRef.current?.close(); } catch { /* ignore */ }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const showBanner = connState === "offline" && !dismissed;

  if (!showBanner) return null;

  return (
    <div className={cn(
      "fixed top-0 left-0 right-0 z-50 flex items-center justify-between gap-3 px-4 py-2.5",
      "bg-amber-500/90 backdrop-blur text-amber-950 text-sm font-medium shadow-lg"
    )}>
      <div className="flex items-center gap-2">
        <WifiOff className="w-4 h-4 flex-shrink-0" />
        <span>API server offline — live data paused. Reconnecting…</span>
      </div>
      <div className="flex items-center gap-2 flex-shrink-0">
        <button
          onClick={() => { clearTimer(); connect(); }}
          className="flex items-center gap-1 text-xs underline underline-offset-2 hover:opacity-80"
          aria-label="Reconnect now"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          Retry
        </button>
        <button
          onClick={() => setDismissed(true)}
          className="ml-1 text-xs hover:opacity-80"
          aria-label="Dismiss"
        >
          ✕
        </button>
      </div>
    </div>
  );
}
