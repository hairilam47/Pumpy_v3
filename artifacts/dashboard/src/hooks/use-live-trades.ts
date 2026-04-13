import { useEffect, useRef, useState, useCallback } from "react";

export interface LiveTrade {
  id: string;
  mint: string;
  tokenName?: string;
  tokenSymbol?: string;
  side: "BUY" | "SELL";
  amountSol: number;
  price?: number;
  status: string;
  strategy?: string;
  signature?: string;
  pnlSol?: number;
  createdAt: string;
  executedAt?: string;
}

interface UseliveTrades {
  trades: LiveTrade[];
  connected: boolean;
  error: string | null;
}

const MAX_LIVE_TRADES = 100;

export function useLiveTrades(): UseliveTrades {
  const [trades, setTrades] = useState<LiveTrade[]>([]);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mounted = useRef(true);

  const connect = useCallback(() => {
    if (!mounted.current) return;

    try {
      const wsProtocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const host = window.location.host;
      const url = `${wsProtocol}//${host}/api/bot/stream`;

      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        if (!mounted.current) { ws.close(); return; }
        setConnected(true);
        setError(null);
      };

      ws.onmessage = (evt) => {
        if (!mounted.current) return;
        try {
          const update = JSON.parse(evt.data) as Partial<LiveTrade>;
          // Require at minimum an id (normalized from gRPC order_id)
          if (!update.id) return;

          const trade: LiveTrade = {
            id: update.id,
            mint: update.mint ?? "",
            tokenName: update.tokenName,
            tokenSymbol: update.tokenSymbol,
            side: update.side ?? "BUY",
            amountSol: update.amountSol ?? 0,
            price: update.price,
            status: update.status ?? "Pending",
            strategy: update.strategy,
            signature: update.signature,
            pnlSol: update.pnlSol,
            createdAt: update.createdAt ?? new Date().toISOString(),
            executedAt: update.executedAt,
          };

          setTrades((prev) => {
            const idx = prev.findIndex((t) => t.id === trade.id);
            if (idx !== -1) {
              const next = [...prev];
              next[idx] = trade;
              return next;
            }
            return [trade, ...prev].slice(0, MAX_LIVE_TRADES);
          });
        } catch {
          // malformed message — ignore
        }
      };

      ws.onerror = () => {
        if (!mounted.current) return;
        setConnected(false);
        setError("WebSocket error");
      };

      ws.onclose = () => {
        if (!mounted.current) return;
        setConnected(false);
        reconnectTimer.current = setTimeout(() => {
          if (mounted.current) connect();
        }, 3000);
      };
    } catch (e) {
      setError(e instanceof Error ? e.message : "WebSocket failed");
      reconnectTimer.current = setTimeout(() => {
        if (mounted.current) connect();
      }, 5000);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    connect();
    return () => {
      mounted.current = false;
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      wsRef.current?.close();
    };
  }, [connect]);

  return { trades, connected, error };
}
