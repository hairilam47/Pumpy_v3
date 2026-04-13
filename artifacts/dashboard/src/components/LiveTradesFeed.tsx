import { useLiveTrades, type LiveTrade } from "@/hooks/use-live-trades";
import { cn, formatSol, formatAge, shortenAddress } from "@/lib/utils";
import { Wifi, WifiOff, ArrowUpRight, ArrowDownRight } from "lucide-react";

function StatusChip({ status }: { status: string }) {
  const cfg: Record<string, string> = {
    Executed: "bg-green-400/10 text-green-400",
    Pending: "bg-amber-400/10 text-amber-400",
    Executing: "bg-blue-400/10 text-blue-400",
    Failed: "bg-red-400/10 text-red-400",
    Cancelled: "bg-muted text-muted-foreground",
  };
  return (
    <span className={cn("px-2 py-0.5 rounded text-xs font-medium", cfg[status] ?? "bg-muted text-muted-foreground")}>
      {status}
    </span>
  );
}

function TradeRow({ trade }: { trade: LiveTrade }) {
  const isBuy = trade.side === "BUY";
  return (
    <tr className="border-b border-border/50 hover:bg-secondary/20 transition-colors text-xs animate-in fade-in slide-in-from-top-1 duration-300">
      <td className="py-2.5 px-4">
        <div className="flex items-center gap-1.5">
          {isBuy
            ? <ArrowUpRight className="w-3.5 h-3.5 text-green-400" />
            : <ArrowDownRight className="w-3.5 h-3.5 text-red-400" />}
          <span className={cn("font-semibold", isBuy ? "text-green-400" : "text-red-400")}>
            {trade.side}
          </span>
        </div>
      </td>
      <td className="py-2.5 px-4">
        <div className="flex flex-col">
          <span className="font-medium text-foreground">{trade.tokenSymbol || "—"}</span>
          <span className="text-muted-foreground font-mono text-xs">{shortenAddress(trade.mint, 4)}</span>
        </div>
      </td>
      <td className="py-2.5 px-4 text-right tabular-nums">{formatSol(trade.amountSol, 4)}</td>
      <td className="py-2.5 px-4">
        <StatusChip status={trade.status} />
      </td>
      <td className="py-2.5 px-4 text-muted-foreground capitalize">{trade.strategy ?? "—"}</td>
      <td className="py-2.5 px-4 text-right tabular-nums">
        {trade.pnlSol != null ? (
          <span className={cn(trade.pnlSol >= 0 ? "text-green-400" : "text-red-400")}>
            {trade.pnlSol >= 0 ? "+" : ""}{trade.pnlSol.toFixed(4)} SOL
          </span>
        ) : "—"}
      </td>
      <td className="py-2.5 px-4 text-right text-muted-foreground">{formatAge(trade.createdAt)}</td>
    </tr>
  );
}

export default function LiveTradesFeed() {
  const { trades, connected, error } = useLiveTrades();

  return (
    <div className="bg-card border border-border rounded-xl overflow-hidden">
      <div className="flex items-center justify-between px-5 py-3 border-b border-border">
        <h2 className="text-sm font-semibold">Live Trade Feed</h2>
        <div className="flex items-center gap-2 text-xs">
          {connected ? (
            <>
              <span className="w-2 h-2 rounded-full bg-green-400 live-pulse" />
              <Wifi className="w-3.5 h-3.5 text-green-400" />
              <span className="text-green-400">Live</span>
            </>
          ) : (
            <>
              <WifiOff className="w-3.5 h-3.5 text-muted-foreground" />
              <span className="text-muted-foreground">{error ? "Error" : "Connecting..."}</span>
            </>
          )}
          <span className="text-muted-foreground ml-1">({trades.length})</span>
        </div>
      </div>

      {trades.length === 0 ? (
        <div className="py-10 text-center text-sm text-muted-foreground">
          {connected
            ? "Waiting for trades..."
            : "Connecting to live feed..."}
        </div>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-xs">
            <thead>
              <tr className="text-muted-foreground border-b border-border bg-secondary/20">
                <th className="text-left py-2.5 px-4 font-medium">Side</th>
                <th className="text-left py-2.5 px-4 font-medium">Token</th>
                <th className="text-right py-2.5 px-4 font-medium">Amount</th>
                <th className="text-left py-2.5 px-4 font-medium">Status</th>
                <th className="text-left py-2.5 px-4 font-medium">Strategy</th>
                <th className="text-right py-2.5 px-4 font-medium">PnL</th>
                <th className="text-right py-2.5 px-4 font-medium">Time</th>
              </tr>
            </thead>
            <tbody>
              {trades.map((t) => <TradeRow key={t.id} trade={t} />)}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
